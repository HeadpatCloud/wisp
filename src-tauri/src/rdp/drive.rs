use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, ReadDir};
use std::io::{self, ErrorKind, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, Characteristics, ClientDriveQueryDirectoryResponse,
    ClientDriveQueryInformationResponse, ClientDriveQueryVolumeInformationResponse,
    ClientDriveSetInformationResponse, CreateDisposition, CreateOptions, DesiredAccess,
    DeviceCloseRequest, DeviceCloseResponse, DeviceControlRequest, DeviceControlResponse,
    DeviceCreateRequest, DeviceCreateResponse, DeviceIoRequest, DeviceIoResponse,
    DeviceReadRequest, DeviceReadResponse, DeviceWriteRequest, DeviceWriteResponse,
    FileAttributeTagInformation, FileAttributes,
    FileBasicInformation, FileBothDirectoryInformation, FileDirectoryInformation,
    FileFsAttributeInformation, FileFsDeviceInformation, FileFsFullSizeInformation,
    FileFsSizeInformation, FileFsVolumeInformation, FileFullDirectoryInformation,
    FileInformationClass, FileInformationClassLevel, FileNamesInformation, FileRenameInformation,
    FileStandardInformation, FileSystemAttributes, FileSystemInformationClass,
    FileSystemInformationClassLevel, Information, NtStatus, ServerDeviceAnnounceResponse,
    ServerDriveIoRequest, ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest,
    ServerDriveQueryVolumeInformationRequest, ServerDriveSetInformationRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_svc::SvcMessage;

use super::share::{self, Denied};

const MAX_OPEN: usize = 1024;
const MAX_READ: u32 = 1 << 20;
const INVALID_HANDLE: u32 = 0xC000_0008;
const DISK_FULL: u32 = 0xC000_007F;
const FILE_IS_A_DIRECTORY: u32 = 0xC000_00BA;
const TOO_MANY_OPENED_FILES: u32 = 0xC000_011F;

const FILE_SUPERSEDED: u8 = 0;
const FILE_OPENED: u8 = 1;
const FILE_CREATED: u8 = 2;
const FILE_OVERWRITTEN: u8 = 3;

const BYTES_PER_SECTOR: u32 = 512;
const SECTORS_PER_UNIT: u32 = 8;
const SERIAL: u32 = 0x5749_5350;
const FILE_DEVICE_DISK: u32 = 0x07;
// 1970-01-01 in 100 ns steps since 1601-01-01.
const UNIX_IN_FILETIME: i64 = 116_444_736_000_000_000;

#[derive(Debug)]
pub(crate) struct Drive {
    root: PathBuf,
    name: String,
    open: HashMap<u32, Open>,
    last_id: u32,
    max_open: usize,
}

impl_as_any!(Drive);

#[derive(Debug)]
struct Open {
    // The entry in its folder, a link as itself: what a delete or a rename acts on.
    path: PathBuf,
    // What a link points to, inside the share.
    target: Option<PathBuf>,
    // A folder has no file and is read by its path, or its target.
    file: Option<File>,
    writable: bool,
    delete: bool,
    listing: Option<Listing>,
}

#[derive(Debug)]
struct Listing {
    entries: ReadDir,
    pattern: String,
}

enum Place {
    Found(PathBuf),
    // Nothing is there, and the folder it would be in is inside the share.
    Free(PathBuf),
}

impl Drive {
    // `root` is canonical; `name` is the share's, which is also the volume label.
    pub(crate) fn new(root: PathBuf, name: String) -> Drive {
        Drive { root, name, open: HashMap::new(), last_id: 0, max_open: open_limit() }
    }

    fn insert(&mut self, open: Open) -> u32 {
        // Create requests carry the file id 0, so no handle gets it.
        loop {
            self.last_id = self.last_id.wrapping_add(1);
            if self.last_id != 0 && !self.open.contains_key(&self.last_id) {
                break;
            }
        }
        self.open.insert(self.last_id, open);
        self.last_id
    }

    fn known(&self, file_id: u32) -> Result<(), NtStatus> {
        self.open.contains_key(&file_id).then_some(()).ok_or(NtStatus::from(INVALID_HANDLE))
    }

    fn place(&self, path: &str) -> Result<Place, NtStatus> {
        if let Ok(found) = share::resolve(&self.root, path, false) {
            return Ok(Place::Found(found));
        }
        match share::resolve(&self.root, path, true) {
            Ok(free) if vacant(&free) => Ok(Place::Free(free)),
            // Something is there that `resolve` does not follow: a link out of the share, or an
            // entry this user may not read.
            Ok(_) => Err(NtStatus::ACCESS_DENIED),
            Err(Denied) => Err(self.refusal(path)),
        }
    }

    // Why `resolve` refused a path: not found where the part of it that exists is a folder in the
    // share and the next part is missing, refused otherwise, so that nothing outside shows through.
    fn refusal(&self, path: &str) -> NtStatus {
        let Ok(parts) = share::parts(path) else { return NtStatus::ACCESS_DENIED };
        let Ok(mut at) = share::resolve(&self.root, "", false) else {
            return NtStatus::NO_SUCH_FILE;
        };
        let mut prefix = String::new();
        for part in parts {
            prefix.push('\\');
            prefix.push_str(part);
            match share::resolve(&self.root, &prefix, false) {
                Ok(found) => at = found,
                Err(Denied) if !at.is_dir() => return NtStatus::NO_SUCH_FILE,
                Err(Denied) => {
                    return match share::resolve(&self.root, &prefix, true) {
                        Ok(free) if vacant(&free) => NtStatus::NO_SUCH_FILE,
                        _ => NtStatus::ACCESS_DENIED,
                    };
                }
            }
        }
        NtStatus::ACCESS_DENIED
    }

    fn create(&mut self, req: DeviceCreateRequest) -> SvcMessage {
        let (status, file_id, information) = match self.opened(&req) {
            Ok((file_id, information)) => (NtStatus::SUCCESS, file_id, information),
            Err(status) => (status, 0, 0),
        };
        SvcMessage::from(RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
            file_id,
            information: Information::from_bits_retain(information),
        }))
    }

    fn opened(&mut self, req: &DeviceCreateRequest) -> Result<(u32, u8), NtStatus> {
        if self.open.len() >= self.max_open {
            return Err(NtStatus::from(TOO_MANY_OPENED_FILES));
        }
        let options = &req.create_options;
        let folder = options.contains(CreateOptions::FILE_DIRECTORY_FILE);
        let delete = options.contains(CreateOptions::FILE_DELETE_ON_CLOSE);
        // A handle that may write keeps programs here from opening the file and denying writes.
        let write = req.desired_access.intersects(
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
                | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY
                | DesiredAccess::GENERIC_WRITE
                | DesiredAccess::GENERIC_ALL
                | DesiredAccess::MAXIMUM_ALLOWED,
        );
        let (path, target, file, information) = match self.place(&req.path)? {
            Place::Found(path) => {
                let information = match req.create_disposition {
                    CreateDisposition::FILE_CREATE => return Err(NtStatus::OBJECT_NAME_COLLISION),
                    CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OPEN_IF => FILE_OPENED,
                    CreateDisposition::FILE_OVERWRITE | CreateDisposition::FILE_OVERWRITE_IF => {
                        FILE_OVERWRITTEN
                    }
                    CreateDisposition::FILE_SUPERSEDE => FILE_SUPERSEDED,
                    _ => return Err(NtStatus::NOT_SUPPORTED),
                };
                let is_dir = path.is_dir();
                let file_wanted = options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE);
                if is_dir && (file_wanted || information != FILE_OPENED) {
                    return Err(NtStatus::from(FILE_IS_A_DIRECTORY));
                }
                if !is_dir && folder {
                    return Err(NtStatus::NOT_A_DIRECTORY);
                }
                if delete && path == self.root {
                    return Err(NtStatus::ACCESS_DENIED);
                }
                // `place` followed it, so a link here points inside the share. Looked at before the
                // file is opened, which can cut it.
                let link = match share::resolve(&self.root, &req.path, true) {
                    Ok(entry) if is_link(&entry) => Some(listed(&entry).map_err(|e| status(&e))?),
                    _ => None,
                };
                let truncate = information != FILE_OPENED;
                let file = if is_dir { None } else { Some(open_file(&path, truncate, write)?) };
                match link {
                    Some(entry) => (entry, Some(path), file, information),
                    None => (path, None, file, information),
                }
            }
            Place::Free(path) => {
                match req.create_disposition {
                    CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OVERWRITE => {
                        return Err(NtStatus::NO_SUCH_FILE);
                    }
                    CreateDisposition::FILE_SUPERSEDE
                    | CreateDisposition::FILE_CREATE
                    | CreateDisposition::FILE_OPEN_IF
                    | CreateDisposition::FILE_OVERWRITE_IF => {}
                    _ => return Err(NtStatus::NOT_SUPPORTED),
                }
                let file = if folder {
                    fs::create_dir(&path).map_err(|e| status(&e))?;
                    None
                } else {
                    // `create_new` follows no link that appeared there since `place` looked.
                    let file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(|e| status(&e))?;
                    Some((file, write))
                };
                (path, None, file, FILE_CREATED)
            }
        };
        let (file, writable) = match file {
            Some((file, writable)) => (Some(file), writable),
            None => (None, false),
        };
        let id = self.insert(Open { path, target, file, writable, delete, listing: None });
        Ok((id, information))
    }

    fn close(&mut self, req: DeviceCloseRequest) -> SvcMessage {
        let status = match self.open.remove(&req.device_io_request.file_id) {
            None => NtStatus::from(INVALID_HANDLE),
            Some(Open { path, target, file, delete, listing, .. }) => {
                let folder = file.is_none();
                // Closed first: on Windows an open handle keeps a file or folder in place.
                drop((file, listing));
                if !delete {
                    NtStatus::SUCCESS
                } else {
                    let removed = remove(&path, folder, target.is_some());
                    removed.map_or_else(|e| status(&e), |()| NtStatus::SUCCESS)
                }
            }
        };
        completion(req.device_io_request, status)
    }

    fn read(&mut self, req: DeviceReadRequest) -> SvcMessage {
        let (status, read_data) = match self.read_data(&req) {
            Ok(data) => (NtStatus::SUCCESS, data),
            Err(status) => (status, Vec::new()),
        };
        SvcMessage::from(RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
            read_data,
        }))
    }

    fn read_data(&mut self, req: &DeviceReadRequest) -> Result<Vec<u8>, NtStatus> {
        let open = self.open.get_mut(&req.device_io_request.file_id);
        let open = open.ok_or(NtStatus::from(INVALID_HANDLE))?;
        let file = open.file.as_mut().ok_or(NtStatus::from(FILE_IS_A_DIRECTORY))?;
        let length = req.length.min(MAX_READ);
        let mut data = Vec::with_capacity(length as usize);
        file.seek(SeekFrom::Start(req.offset))
            .and_then(|_| (&*file).take(u64::from(length)).read_to_end(&mut data))
            .map_err(|e| status(&e))?;
        Ok(data)
    }

    fn write(&mut self, req: DeviceWriteRequest) -> SvcMessage {
        let (status, length) = match self.written(&req) {
            Ok(length) => (NtStatus::SUCCESS, length),
            Err(status) => (status, 0),
        };
        SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
            length,
        }))
    }

    fn written(&mut self, req: &DeviceWriteRequest) -> Result<u32, NtStatus> {
        let open = self.open.get_mut(&req.device_io_request.file_id);
        let open = open.ok_or(NtStatus::from(INVALID_HANDLE))?;
        let file = open.file.as_mut().ok_or(NtStatus::from(FILE_IS_A_DIRECTORY))?;
        if !open.writable {
            return Err(NtStatus::ACCESS_DENIED);
        }
        file.seek(SeekFrom::Start(req.offset))
            .and_then(|_| file.write_all(&req.write_data))
            .map_err(|e| status(&e))?;
        Ok(req.write_data.len() as u32)
    }

    fn query_information(&mut self, req: ServerDriveQueryInformationRequest) -> SvcMessage {
        let (status, buffer) = match self.information(&req) {
            Ok(buffer) => (NtStatus::SUCCESS, Some(buffer)),
            Err(status) => (status, None),
        };
        SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
            ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, status),
                buffer,
            },
        ))
    }

    fn information(
        &self,
        req: &ServerDriveQueryInformationRequest,
    ) -> Result<FileInformationClass, NtStatus> {
        let open = self.open.get(&req.device_io_request.file_id);
        let open = open.ok_or(NtStatus::from(INVALID_HANDLE))?;
        let meta = match &open.file {
            Some(file) => file.metadata(),
            None => fs::symlink_metadata(open.target.as_ref().unwrap_or(&open.path)),
        };
        let meta = meta.map_err(|e| status(&e))?;
        Ok(match req.file_info_class_lvl {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => {
                FileInformationClass::Basic(FileBasicInformation {
                    creation_time: filetime(meta.created()),
                    last_access_time: filetime(meta.accessed()),
                    last_write_time: filetime(meta.modified()),
                    change_time: filetime(meta.modified()),
                    file_attributes: attributes(&meta),
                })
            }
            FileInformationClassLevel::FILE_STANDARD_INFORMATION => {
                FileInformationClass::Standard(FileStandardInformation {
                    allocation_size: size(&meta),
                    end_of_file: size(&meta),
                    number_of_links: 1,
                    delete_pending: if open.delete { Boolean::True } else { Boolean::False },
                    directory: if meta.is_dir() { Boolean::True } else { Boolean::False },
                })
            }
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => {
                FileInformationClass::AttributeTag(FileAttributeTagInformation {
                    file_attributes: attributes(&meta),
                    reparse_tag: 0,
                })
            }
            _ => return Err(NtStatus::NOT_SUPPORTED),
        })
    }

    fn query_volume(&mut self, req: ServerDriveQueryVolumeInformationRequest) -> SvcMessage {
        let (status, buffer) = match self.volume(&req) {
            Ok(buffer) => (NtStatus::SUCCESS, Some(buffer)),
            Err(status) => (status, None),
        };
        SvcMessage::from(RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
            ClientDriveQueryVolumeInformationResponse::new(req.device_io_request, status, buffer),
        ))
    }

    fn volume(
        &self,
        req: &ServerDriveQueryVolumeInformationRequest,
    ) -> Result<FileSystemInformationClass, NtStatus> {
        self.known(req.device_io_request.file_id)?;
        Ok(match req.fs_info_class_lvl {
            FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION => {
                let meta = fs::metadata(&self.root).map_err(|e| status(&e))?;
                FileSystemInformationClass::FileFsVolumeInformation(FileFsVolumeInformation {
                    volume_creation_time: filetime(meta.created()),
                    volume_serial_number: SERIAL,
                    supports_objects: Boolean::False,
                    volume_label: self.name.clone(),
                })
            }
            FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION => {
                let space = space(&self.root).map_err(|e| status(&e))?;
                FileSystemInformationClass::FileFsSizeInformation(FileFsSizeInformation {
                    total_alloc_units: units(space.total),
                    available_alloc_units: units(space.caller),
                    sectors_per_alloc_unit: SECTORS_PER_UNIT,
                    bytes_per_sector: BYTES_PER_SECTOR,
                })
            }
            FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => {
                let space = space(&self.root).map_err(|e| status(&e))?;
                FileSystemInformationClass::FileFsFullSizeInformation(FileFsFullSizeInformation {
                    total_alloc_units: units(space.total),
                    caller_available_alloc_units: units(space.caller),
                    actual_available_alloc_units: units(space.free),
                    sectors_per_alloc_unit: SECTORS_PER_UNIT,
                    bytes_per_sector: BYTES_PER_SECTOR,
                })
            }
            FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION => {
                FileSystemInformationClass::FileFsAttributeInformation(FileFsAttributeInformation {
                    file_system_attributes: FileSystemAttributes::FILE_CASE_PRESERVED_NAMES
                        | FileSystemAttributes::FILE_UNICODE_ON_DISK,
                    max_component_name_len: 255,
                    file_system_name: "NTFS".to_owned(),
                })
            }
            FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION => {
                FileSystemInformationClass::FileFsDeviceInformation(FileFsDeviceInformation {
                    device_type: FILE_DEVICE_DISK,
                    characteristics: Characteristics::empty(),
                })
            }
            _ => return Err(NtStatus::NOT_SUPPORTED),
        })
    }

    fn query_directory(&mut self, req: ServerDriveQueryDirectoryRequest) -> SvcMessage {
        let (status, buffer) = match self.entry(&req) {
            Ok(buffer) => (NtStatus::SUCCESS, Some(buffer)),
            Err(status) => (status, None),
        };
        SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
            ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
                buffer,
            },
        ))
    }

    fn entry(
        &mut self,
        req: &ServerDriveQueryDirectoryRequest,
    ) -> Result<FileInformationClass, NtStatus> {
        let id = req.device_io_request.file_id;
        self.known(id)?;
        let initial = req.initial_query != 0;
        if initial {
            // A folder and a pattern: `\sub\*.txt`, or `\name` for one entry of the root.
            let (folder, pattern) = req.path.rsplit_once(['\\', '/']).unwrap_or(("", &req.path));
            let found = share::resolve(&self.root, folder, false);
            let found = found.map_err(|Denied| self.refusal(folder))?;
            let entries = fs::read_dir(found).map_err(|e| status(&e))?;
            if let Some(open) = self.open.get_mut(&id) {
                open.listing = Some(Listing { entries, pattern: pattern.to_owned() });
            }
        }
        let listing = self.open.get_mut(&id).and_then(|open| open.listing.as_mut());
        let Some((name, meta)) = listing.and_then(Listing::next_match) else {
            return Err(if initial { NtStatus::NO_SUCH_FILE } else { NtStatus::NO_MORE_FILES });
        };
        let (created, accessed) = (filetime(meta.created()), filetime(meta.accessed()));
        let (written, size, attributes) =
            (filetime(meta.modified()), size(&meta), attributes(&meta));
        Ok(match req.file_info_class_lvl {
            FileInformationClassLevel::FILE_DIRECTORY_INFORMATION => {
                FileInformationClass::Directory(FileDirectoryInformation::new(
                    created, accessed, written, written, size, attributes, name,
                ))
            }
            FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => {
                FileInformationClass::FullDirectory(FileFullDirectoryInformation::new(
                    created, accessed, written, written, size, attributes, name,
                ))
            }
            FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => {
                FileInformationClass::BothDirectory(FileBothDirectoryInformation::new(
                    created, accessed, written, written, size, attributes, name,
                ))
            }
            FileInformationClassLevel::FILE_NAMES_INFORMATION => {
                FileInformationClass::Names(FileNamesInformation::new(name))
            }
            _ => return Err(NtStatus::NOT_SUPPORTED),
        })
    }

    fn set_information(&mut self, req: ServerDriveSetInformationRequest) -> SvcMessage {
        let status = self.set(&req);
        match ClientDriveSetInformationResponse::new(&req, status) {
            Ok(reply) => SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(reply)),
            // Only a length past u32::MAX fails, which no channel message can carry.
            Err(_) => completion(req.device_io_request, status),
        }
    }

    fn set(&mut self, req: &ServerDriveSetInformationRequest) -> NtStatus {
        let id = req.device_io_request.file_id;
        if let FileInformationClass::Rename(info) = &req.set_buffer {
            return self.rename(id, info);
        }
        let Some(open) = self.open.get_mut(&id) else { return NtStatus::from(INVALID_HANDLE) };
        match &req.set_buffer {
            // Accepted without applying: times and attributes stay as this system set them.
            FileInformationClass::Basic(_) => NtStatus::SUCCESS,
            FileInformationClass::EndOfFile(info) => resize(open, info.end_of_file, false),
            FileInformationClass::Allocation(info) => resize(open, info.allocation_size, true),
            FileInformationClass::Disposition(info) => {
                if info.delete_pending == 0 {
                    open.delete = false;
                    return NtStatus::SUCCESS;
                }
                if open.path == self.root {
                    return NtStatus::ACCESS_DENIED;
                }
                // A link to a folder holds nothing itself.
                if open.file.is_none() && open.target.is_none() {
                    match fs::read_dir(&open.path).map(|mut entries| entries.next().is_some()) {
                        Ok(true) => return NtStatus::DIRECTORY_NOT_EMPTY,
                        Ok(false) => {}
                        Err(e) => return status(&e),
                    }
                }
                open.delete = true;
                NtStatus::SUCCESS
            }
            _ => NtStatus::NOT_SUPPORTED,
        }
    }

    fn rename(&mut self, id: u32, info: &FileRenameInformation) -> NtStatus {
        let Some(from) = self.open.get(&id).map(|open| open.path.clone()) else {
            return NtStatus::from(INVALID_HANDLE);
        };
        if from == self.root {
            return NtStatus::ACCESS_DENIED;
        }
        let to = match share::resolve(&self.root, &info.file_name, true) {
            Ok(to) => to,
            Err(Denied) => return self.refusal(&info.file_name),
        };
        // A name that differs only in case is the same file on Windows and macOS.
        let taken = !vacant(&to) && !same_entry(&from, &to).is_ok_and(|same| same);
        if taken && info.replace_if_exists == Boolean::False {
            return NtStatus::OBJECT_NAME_COLLISION;
        }
        // NTFS refuses to replace a file another handle holds, and that handle's pending delete
        // would take the moved file; std's rename on Windows replaces it all the same.
        let held = |open: &Open| {
            let mut paths = std::iter::once(&open.path).chain(&open.target);
            paths.any(|path| same_entry(path, &to).is_ok_and(|same| same))
        };
        if taken && self.open.iter().any(|(&other, open)| other != id && held(open)) {
            return NtStatus::ACCESS_DENIED;
        }
        if let Err(e) = fs::rename(&from, &to) {
            return status(&e);
        }
        // Every handle on the entry or inside it, this one or another, has it at its new name now.
        for open in self.open.values_mut() {
            for path in std::iter::once(&mut open.path).chain(open.target.as_mut()) {
                let Ok(rest) = path.strip_prefix(&from) else { continue };
                *path = if rest.as_os_str().is_empty() { to.clone() } else { to.join(rest) };
            }
        }
        NtStatus::SUCCESS
    }
}

// The session has ended: what the server would have deleted at its closes goes now.
impl Drop for Drive {
    fn drop(&mut self) {
        // Every handle is closed first, as an open handle keeps a file or folder in place.
        let opens = self.open.drain().map(|(_, open)| open);
        let mut pending: Vec<(PathBuf, bool, bool)> = opens
            .filter(|open| open.delete)
            .map(|open| (open.path, open.file.is_none(), open.target.is_some()))
            .collect();
        // What is inside a folder goes before the folder.
        pending.sort_by_key(|(path, ..)| std::cmp::Reverse(path.components().count()));
        for (path, folder, link) in pending {
            // Nobody is left to tell of a delete that fails.
            let _ = remove(&path, folder, link);
        }
    }
}

impl RdpdrBackend for Drive {
    fn handle_server_device_announce_response(
        &mut self,
        _pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<()> {
        Ok(())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        let reply = match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(r) => self.create(r),
            ServerDriveIoRequest::DeviceCloseRequest(r) => self.close(r),
            ServerDriveIoRequest::DeviceReadRequest(r) => self.read(r),
            ServerDriveIoRequest::DeviceWriteRequest(r) => self.write(r),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(r) => {
                self.query_information(r)
            }
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(r) => {
                self.query_volume(r)
            }
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => self.query_directory(r),
            ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => self.set_information(r),
            ServerDriveIoRequest::DeviceControlRequest(r) => {
                let status = self.known(r.header.file_id).err().unwrap_or(NtStatus::NOT_SUPPORTED);
                SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
                    device_io_reply: DeviceIoResponse::new(r.header, status),
                    output_buffer: None,
                }))
            }
            ServerDriveIoRequest::ServerDriveLockControlRequest(r) => {
                let known = self.known(r.device_io_request.file_id);
                completion(r.device_io_request, known.err().unwrap_or(NtStatus::NOT_SUPPORTED))
            }
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(r) => {
                match self.known(r.device_io_request.file_id) {
                    // Left pending; the server ends it by closing the handle.
                    Ok(()) => return Ok(Vec::new()),
                    Err(status) => completion(r.device_io_request, status),
                }
            }
        };
        Ok(vec![reply])
    }
}

impl Listing {
    // An entry that cannot be read is left out, so that the rest of the listing still comes.
    fn next_match(&mut self) -> Option<(String, fs::Metadata)> {
        for entry in self.entries.by_ref() {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if !matches(&self.pattern, &name) {
                continue;
            }
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                return Some((name, meta));
            }
        }
        None
    }
}

// A completion with nothing after the status, laid out as a close's.
fn completion(req: DeviceIoRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(req, status),
    }))
}

fn status(e: &io::Error) -> NtStatus {
    // Rust gives these two no kind of their own.
    #[cfg(unix)]
    if matches!(e.raw_os_error(), Some(libc::EMFILE | libc::ENFILE)) {
        return NtStatus::from(TOO_MANY_OPENED_FILES);
    }
    match e.kind() {
        ErrorKind::NotFound => NtStatus::NO_SUCH_FILE,
        ErrorKind::AlreadyExists => NtStatus::OBJECT_NAME_COLLISION,
        ErrorKind::NotADirectory => NtStatus::NOT_A_DIRECTORY,
        ErrorKind::IsADirectory => NtStatus::from(FILE_IS_A_DIRECTORY),
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => NtStatus::ACCESS_DENIED,
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => NtStatus::from(DISK_FULL),
        ErrorKind::DirectoryNotEmpty => NtStatus::DIRECTORY_NOT_EMPTY,
        _ => NtStatus::UNSUCCESSFUL,
    }
}

#[cfg(windows)]
fn open_limit() -> usize {
    MAX_OPEN
}

// Each handle holds at most one descriptor, its file or its listing, so a server holds at most a
// quarter of what the process may open and the rest stays for its other sessions and the webview.
#[cfg(unix)]
fn open_limit() -> usize {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // It fails only for an unknown resource, and then no handle opens.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return 0;
    }
    handle_cap(limit.rlim_cur)
}

#[cfg(unix)]
fn handle_cap(descriptors: libc::rlim_t) -> usize {
    usize::try_from(descriptors / 4).map_or(MAX_OPEN, |quarter| quarter.min(MAX_OPEN))
}

fn vacant(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|e| e.kind() == ErrorKind::NotFound)
}

// Windows removes a link to a folder as a folder, the others every link as a file.
fn remove(path: &Path, folder: bool, link: bool) -> io::Result<()> {
    if folder && !(link && cfg!(unix)) {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

// `entry` with its name as its folder lists it. A host that ignores case takes the server's
// spelling too, and handles are matched by their paths as spelled. An 8.3 name, another Unicode
// normalisation or a case pair the host folds otherwise than `to_lowercase` matches no listed
// name, and the entry is then looked for among them all.
fn listed(entry: &Path) -> io::Result<PathBuf> {
    let missing = || io::Error::from(ErrorKind::NotFound);
    let parent = entry.parent().ok_or_else(missing)?;
    let name = entry.file_name().ok_or_else(missing)?.to_string_lossy().to_lowercase();
    let found: Vec<PathBuf> =
        fs::read_dir(parent)?.filter_map(Result::ok).map(|found| found.path()).collect();
    let same = |found: &&PathBuf| same_entry(found, entry).is_ok_and(|same| same);
    let spelled = |found: &&PathBuf| {
        found.file_name().is_some_and(|it| it.to_string_lossy().to_lowercase() == name)
    };
    let mut candidates = found.iter().filter(spelled);
    let by_name = candidates.find(same);
    by_name.or_else(|| found.iter().find(same)).cloned().ok_or_else(missing)
}

// On Windows a junction is one too.
fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

// Whether two paths name the same entry of a folder, a link as itself and not what it points to.
#[cfg(unix)]
fn same_entry(a: &Path, b: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;

    let (a, b) = (fs::symlink_metadata(a)?, fs::symlink_metadata(b)?);
    Ok((a.dev(), a.ino()) == (b.dev(), b.ino()))
}

#[cfg(windows)]
fn same_entry(a: &Path, b: &Path) -> io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let id = |path: &Path| -> io::Result<(u32, u32, u32)> {
        let entry = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(entry.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
    };
    Ok(id(a)? == id(b)?)
}

// Read only, unless the server asked to write or the file is to be cut: then read and write where
// this system allows it, otherwise read only.
fn open_file(path: &Path, truncate: bool, write: bool) -> Result<(File, bool), NtStatus> {
    if !write && !truncate {
        return File::open(path).map(|file| (file, false)).map_err(|e| status(&e));
    }
    match OpenOptions::new().read(true).write(true).truncate(truncate).open(path) {
        Ok(file) => Ok((file, write)),
        Err(_) if !truncate => File::open(path).map(|file| (file, false)).map_err(|e| status(&e)),
        Err(e) => Err(status(&e)),
    }
}

// The end of file is set as given; an allocation only cuts a file that is longer.
fn resize(open: &mut Open, size: i64, allocation: bool) -> NtStatus {
    let Some(file) = &open.file else { return NtStatus::from(FILE_IS_A_DIRECTORY) };
    if !open.writable {
        return NtStatus::ACCESS_DENIED;
    }
    let Ok(size) = u64::try_from(size) else { return NtStatus::UNSUCCESSFUL };
    let resized = if allocation {
        file.metadata().and_then(|meta| if size < meta.len() { file.set_len(size) } else { Ok(()) })
    } else {
        file.set_len(size)
    };
    resized.map_or_else(|e| status(&e), |()| NtStatus::SUCCESS)
}

fn filetime(time: io::Result<SystemTime>) -> i64 {
    let Ok(time) = time else { return 0 };
    let steps = |since: std::time::Duration| i64::try_from(since.as_nanos() / 100);
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => steps(after).map_or(i64::MAX, |n| UNIX_IN_FILETIME.saturating_add(n)),
        Err(before) => steps(before.duration()).map_or(0, |n| (UNIX_IN_FILETIME - n).max(0)),
    }
}

fn size(meta: &fs::Metadata) -> i64 {
    i64::try_from(meta.len()).unwrap_or(i64::MAX)
}

fn attributes(meta: &fs::Metadata) -> FileAttributes {
    let mut attributes = if meta.is_dir() {
        FileAttributes::FILE_ATTRIBUTE_DIRECTORY
    } else {
        FileAttributes::FILE_ATTRIBUTE_ARCHIVE
    };
    if meta.permissions().readonly() {
        attributes |= FileAttributes::FILE_ATTRIBUTE_READONLY;
    }
    attributes
}

#[derive(Clone, Copy)]
enum Wildcard {
    Char(char),
    Star,
    // The DOS_STAR, DOS_QM and DOS_DOT of FsRtlIsNameInExpression.
    DosStar,
    DosQm,
    DosDot,
}

// Windows forwards a pattern as it was typed, `*.` included, so it is matched as FindFirstFile
// matches it in a local folder: `*` before `.`, every `?`, and `.` before `?`, `*` or the end are
// the DOS wildcards. Without regard to ASCII case. Each pair of positions is looked at once, so
// that a pattern full of `*` cannot take exponential time.
fn matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let wildcards: Vec<Wildcard> = pattern
        .iter()
        .enumerate()
        .map(|(at, &c)| match (c, pattern.get(at + 1)) {
            ('*', Some('.')) => Wildcard::DosStar,
            ('*', _) => Wildcard::Star,
            ('?', _) => Wildcard::DosQm,
            ('.', None | Some('?' | '*')) => Wildcard::DosDot,
            (c, _) => Wildcard::Char(c),
        })
        .collect();
    let name: Vec<char> = name.chars().collect();
    let last_dot = name.iter().rposition(|&c| c == '.');
    // rest[n]: whether the wildcards from the one at hand on match name[n..].
    let mut rest = vec![false; name.len() + 1];
    rest[name.len()] = true;
    // Whether every wildcard after the one at hand is a DOS_QM.
    let mut only_qms = true;
    for wildcard in wildcards.iter().rev() {
        let next = rest;
        rest = vec![false; name.len() + 1];
        for n in (0..=name.len()).rev() {
            rest[n] = match (*wildcard, name.get(n)) {
                (Wildcard::Char(c), Some(here)) => c.eq_ignore_ascii_case(here) && next[n + 1],
                (Wildcard::Char(_), None) => false,
                (Wildcard::Star, Some(_)) => next[n] || rest[n + 1],
                (Wildcard::Star, None) => next[n],
                // Up to the last dot of the name, not past it.
                (Wildcard::DosStar, Some(_)) if Some(n) != last_dot => next[n] || rest[n + 1],
                (Wildcard::DosStar, _) => next[n],
                (Wildcard::DosQm, None) => next[n],
                // A dot that ends the name is taken when only DOS_QMs follow.
                (Wildcard::DosQm, Some('.')) => next[n] || (only_qms && n + 1 == name.len()),
                (Wildcard::DosQm, Some(_)) => next[n + 1],
                (Wildcard::DosDot, None) => next[n],
                (Wildcard::DosDot, Some('.')) => next[n + 1],
                (Wildcard::DosDot, Some(_)) => false,
            };
        }
        only_qms &= matches!(wildcard, Wildcard::DosQm);
    }
    rest[0]
}

fn units(bytes: u64) -> i64 {
    i64::try_from(bytes / u64::from(BYTES_PER_SECTOR * SECTORS_PER_UNIT)).unwrap_or(i64::MAX)
}

// Bytes of the volume: in all, free to this user and free in all.
struct Space {
    total: u64,
    caller: u64,
    free: u64,
}

#[cfg(windows)]
fn space(folder: &Path) -> io::Result<Space> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // With a trailing `\`, which a UNC folder needs.
    let wide: Vec<u16> = folder.join("").as_os_str().encode_wide().chain([0]).collect();
    let (mut caller, mut total, mut free) = (0, 0, 0);
    if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut caller, &mut total, &mut free) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Space { total, caller, free })
}

#[cfg(unix)]
fn space(folder: &Path) -> io::Result<Space> {
    use std::os::unix::ffi::OsStrExt as _;

    let path = std::ffi::CString::new(folder.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    let unit = u64::from(stat.f_frsize);
    Ok(Space {
        total: u64::from(stat.f_blocks).saturating_mul(unit),
        caller: u64::from(stat.f_bavail).saturating_mul(unit),
        free: u64::from(stat.f_bfree).saturating_mul(unit),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    use ironrdp_rdpdr::pdu::efs::{
        AnyIoCtlCode, FileAllocationInformation, FileDispositionInformation,
        FileEndOfFileInformation, MajorFunction, MinorFunction, ServerDriveLockControlRequest,
        ServerDriveNotifyChangeDirectoryRequest, SharedAccess,
    };
    use ironrdp_rdpdr::Rdpdr;
    use ironrdp_svc::SvcProcessor as _;
    use tempfile::TempDir;

    use super::*;

    const DEVICE: u32 = 1;
    const SUCCESS: u32 = 0;
    const NO_MORE_FILES: u32 = 0x8000_0006;
    const UNSUCCESSFUL: u32 = 0xC000_0001;
    const NO_SUCH_FILE: u32 = 0xC000_000F;
    const ACCESS_DENIED: u32 = 0xC000_0022;
    const COLLISION: u32 = 0xC000_0035;
    const NOT_SUPPORTED: u32 = 0xC000_00BB;
    const NOT_EMPTY: u32 = 0xC000_0101;
    const NOT_A_DIRECTORY: u32 = 0xC000_0103;
    const DIRECTORY: u32 = 0x10;
    const ARCHIVE: u32 = 0x20;

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    fn i64_at(bytes: &[u8], at: usize) -> i64 {
        i64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
    }

    fn utf16(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).unwrap()
    }

    fn status_of(reply: &[u8]) -> u32 {
        u32_at(reply, 12)
    }

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        names.sort();
        names
    }

    fn names_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // A share holding `readme.txt`, `notes.txt` and `sub\inner.txt`, next to a folder `outside`
    // holding `secret.txt`.
    struct Rig {
        dir: TempDir,
        root: PathBuf,
        drive: Drive,
        completion: u32,
    }

    impl Rig {
        fn new() -> Rig {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("share");
            fs::create_dir_all(root.join("sub")).unwrap();
            fs::create_dir(dir.path().join("outside")).unwrap();
            fs::write(root.join("readme.txt"), "hello share").unwrap();
            fs::write(root.join("notes.txt"), "notes").unwrap();
            fs::write(root.join("sub").join("inner.txt"), "inner").unwrap();
            fs::write(dir.path().join("outside").join("secret.txt"), "secret").unwrap();
            let root = fs::canonicalize(root).unwrap();
            let drive = Drive::new(root.clone(), "wisp".to_owned());
            Rig { dir, root, drive, completion: 0 }
        }

        fn outside(&self) -> PathBuf {
            self.dir.path().join("outside")
        }

        // A link named `name` in the share to the folder `target`, a junction on Windows.
        fn link_folder(&self, name: &str, target: &Path) {
            let at = self.dir.path().join("share").join(name);
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, &at).unwrap();
            #[cfg(windows)]
            {
                let mut mklink = std::process::Command::new("cmd");
                let made = mklink.args(["/C", "mklink", "/J"]).arg(&at).arg(target).output();
                let made = made.unwrap();
                assert!(made.status.success(), "mklink /J failed: {made:?}");
            }
        }

        // A symbolic link to a file, which Windows makes only in developer mode.
        fn link_file(&self, name: &str, target: &Path) -> bool {
            let at = self.dir.path().join("share").join(name);
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, &at).unwrap();
            #[cfg(windows)]
            if let Err(e) = std::os::windows::fs::symlink_file(target, &at) {
                println!("no symbolic link without developer mode ({e}), test skipped");
                return false;
            }
            true
        }

        fn io(&mut self, file_id: u32, major: MajorFunction, minor: u32) -> DeviceIoRequest {
            self.completion += 1;
            DeviceIoRequest {
                device_id: DEVICE,
                file_id,
                completion_id: self.completion,
                major_function: major,
                minor_function: MinorFunction::from(minor),
            }
        }

        fn replies(&mut self, req: ServerDriveIoRequest) -> Vec<Vec<u8>> {
            let replies = self.drive.handle_drive_io_request(req).expect("the backend failed");
            replies.iter().map(|reply| reply.encode_unframed_pdu().unwrap()).collect()
        }

        // The one completion the request gets, for this device and with its completion id.
        fn send(&mut self, req: ServerDriveIoRequest) -> Vec<u8> {
            let completion = self.completion;
            let mut replies = self.replies(req);
            assert_eq!(replies.len(), 1, "one reply");
            let reply = replies.remove(0);
            assert_eq!(&reply[..4], &[0x72, 0x44, 0x43, 0x49], "a device I/O completion");
            assert_eq!((u32_at(&reply, 4), u32_at(&reply, 8)), (DEVICE, completion));
            reply
        }

        fn create(
            &mut self,
            path: &str,
            disposition: CreateDisposition,
            options: CreateOptions,
        ) -> Vec<u8> {
            let access = DesiredAccess::GENERIC_READ | DesiredAccess::GENERIC_WRITE;
            self.create_for(access, path, disposition, options)
        }

        fn create_for(
            &mut self,
            desired_access: DesiredAccess,
            path: &str,
            disposition: CreateDisposition,
            options: CreateOptions,
        ) -> Vec<u8> {
            let device_io_request = self.io(0, MajorFunction::Create, 0);
            self.send(ServerDriveIoRequest::ServerCreateDriveRequest(DeviceCreateRequest {
                device_io_request,
                desired_access,
                allocation_size: 0,
                file_attributes: FileAttributes::empty(),
                shared_access: SharedAccess::FILE_SHARE_READ | SharedAccess::FILE_SHARE_WRITE,
                create_disposition: disposition,
                create_options: options,
                path: path.to_owned(),
            }))
        }

        fn opened(&mut self, path: &str, options: CreateOptions) -> u32 {
            let reply = self.create(path, CreateDisposition::FILE_OPEN, options);
            assert_eq!(status_of(&reply), SUCCESS, "{path} did not open");
            u32_at(&reply, 16)
        }

        fn file(&mut self, path: &str) -> u32 {
            self.opened(path, CreateOptions::FILE_NON_DIRECTORY_FILE)
        }

        fn folder(&mut self, path: &str) -> u32 {
            self.opened(path, CreateOptions::FILE_DIRECTORY_FILE)
        }

        fn close(&mut self, file_id: u32) -> u32 {
            let device_io_request = self.io(file_id, MajorFunction::Close, 0);
            let close = DeviceCloseRequest { device_io_request };
            status_of(&self.send(ServerDriveIoRequest::DeviceCloseRequest(close)))
        }

        fn read(&mut self, file_id: u32, offset: u64, length: u32) -> (u32, Vec<u8>) {
            let device_io_request = self.io(file_id, MajorFunction::Read, 0);
            let read = DeviceReadRequest { device_io_request, length, offset };
            let reply = self.send(ServerDriveIoRequest::DeviceReadRequest(read));
            let length = u32_at(&reply, 16) as usize;
            assert_eq!(reply.len(), 20 + length);
            (status_of(&reply), reply[20..].to_vec())
        }

        fn write(&mut self, file_id: u32, offset: u64, data: &[u8]) -> (u32, u32) {
            let device_io_request = self.io(file_id, MajorFunction::Write, 0);
            let write_data = data.to_vec();
            let write = DeviceWriteRequest { device_io_request, offset, write_data };
            let reply = self.send(ServerDriveIoRequest::DeviceWriteRequest(write));
            (status_of(&reply), u32_at(&reply, 16))
        }

        fn query(&mut self, file_id: u32, class: u32) -> Vec<u8> {
            let device_io_request = self.io(file_id, MajorFunction::QueryInformation, 0);
            let file_info_class_lvl = FileInformationClassLevel::from(class);
            let query =
                ServerDriveQueryInformationRequest { device_io_request, file_info_class_lvl };
            self.send(ServerDriveIoRequest::ServerDriveQueryInformationRequest(query))
        }

        fn volume(&mut self, file_id: u32, class: u32) -> Vec<u8> {
            let device_io_request = self.io(file_id, MajorFunction::QueryVolumeInformation, 0);
            let fs_info_class_lvl = FileSystemInformationClassLevel::from(class);
            let query =
                ServerDriveQueryVolumeInformationRequest { device_io_request, fs_info_class_lvl };
            self.send(ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(query))
        }

        fn directory(&mut self, file_id: u32, class: u32, initial: bool, path: &str) -> Vec<u8> {
            let device_io_request = self.io(file_id, MajorFunction::DirectoryControl, 1);
            let query = ServerDriveQueryDirectoryRequest {
                device_io_request,
                file_info_class_lvl: FileInformationClassLevel::from(class),
                initial_query: u8::from(initial),
                path: path.to_owned(),
            };
            self.send(ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(query))
        }

        // The names a listing gives in `class`, sorted, or the status of its first answer.
        fn list(&mut self, file_id: u32, class: u32, path: &str) -> Result<Vec<String>, u32> {
            let mut reply = self.directory(file_id, class, true, path);
            if status_of(&reply) != SUCCESS {
                assert_eq!(u32_at(&reply, 16), 0, "a refusal carries no entry");
                return Err(status_of(&reply));
            }
            let mut names = Vec::new();
            while status_of(&reply) == SUCCESS {
                names.push(entry_name(class, &reply[20..]));
                reply = self.directory(file_id, class, false, "");
            }
            assert_eq!(status_of(&reply), NO_MORE_FILES);
            let again = self.directory(file_id, class, false, "");
            assert_eq!(status_of(&again), NO_MORE_FILES);
            names.sort();
            Ok(names)
        }

        fn set(&mut self, file_id: u32, info: FileInformationClass) -> u32 {
            let device_io_request = self.io(file_id, MajorFunction::SetInformation, 0);
            let set = ServerDriveSetInformationRequest { device_io_request, set_buffer: info };
            status_of(&self.send(ServerDriveIoRequest::ServerDriveSetInformationRequest(set)))
        }

        fn rename(&mut self, file_id: u32, to: &str, replace: bool) -> u32 {
            let replace_if_exists = if replace { Boolean::True } else { Boolean::False };
            let info = FileRenameInformation { replace_if_exists, file_name: to.to_owned() };
            self.set(file_id, FileInformationClass::Rename(info))
        }

        fn dispose(&mut self, file_id: u32, delete: bool) -> u32 {
            let info = FileDispositionInformation { delete_pending: u8::from(delete) };
            self.set(file_id, FileInformationClass::Disposition(info))
        }
    }

    // The name in a directory entry of `class`.
    fn entry_name(class: u32, entry: &[u8]) -> String {
        let (length_at, name_at) = match class {
            1 => (60, 64),
            2 => (60, 68),
            3 => (60, 93),
            12 => (8, 12),
            other => panic!("no directory class {other}"),
        };
        let length = u32_at(entry, length_at) as usize;
        assert_eq!(entry.len(), name_at + length);
        utf16(&entry[name_at..])
    }

    #[test]
    fn every_disposition_with_the_target_present() {
        let cases = [
            (CreateDisposition::FILE_SUPERSEDE, SUCCESS, FILE_SUPERSEDED, ""),
            (CreateDisposition::FILE_OPEN, SUCCESS, FILE_OPENED, "hello share"),
            (CreateDisposition::FILE_CREATE, COLLISION, 0, "hello share"),
            (CreateDisposition::FILE_OPEN_IF, SUCCESS, FILE_OPENED, "hello share"),
            (CreateDisposition::FILE_OVERWRITE, SUCCESS, FILE_OVERWRITTEN, ""),
            (CreateDisposition::FILE_OVERWRITE_IF, SUCCESS, FILE_OVERWRITTEN, ""),
        ];
        for (disposition, status, information, left) in cases {
            let mut rig = Rig::new();
            let options = CreateOptions::FILE_NON_DIRECTORY_FILE;
            let reply = rig.create("\\readme.txt", disposition, options);
            assert_eq!((status_of(&reply), reply[20]), (status, information), "{disposition:?}");
            assert_eq!(u32_at(&reply, 16) != 0, status == SUCCESS, "{disposition:?}");
            let content = fs::read_to_string(rig.root.join("readme.txt")).unwrap();
            assert_eq!(content, left, "{disposition:?}");
        }
    }

    #[test]
    fn every_disposition_with_the_target_missing() {
        let cases = [
            (CreateDisposition::FILE_SUPERSEDE, SUCCESS, FILE_CREATED),
            (CreateDisposition::FILE_OPEN, NO_SUCH_FILE, 0),
            (CreateDisposition::FILE_CREATE, SUCCESS, FILE_CREATED),
            (CreateDisposition::FILE_OPEN_IF, SUCCESS, FILE_CREATED),
            (CreateDisposition::FILE_OVERWRITE, NO_SUCH_FILE, 0),
            (CreateDisposition::FILE_OVERWRITE_IF, SUCCESS, FILE_CREATED),
        ];
        for (disposition, status, information) in cases {
            let mut rig = Rig::new();
            let reply = rig.create("/new.txt", disposition, CreateOptions::empty());
            assert_eq!((status_of(&reply), reply[20]), (status, information), "{disposition:?}");
            assert_eq!(rig.root.join("new.txt").is_file(), status == SUCCESS, "{disposition:?}");
            let reply = rig.create("\\missing\\new.txt", disposition, CreateOptions::empty());
            assert_eq!(status_of(&reply), NO_SUCH_FILE, "{disposition:?} in a missing folder");
            assert!(!rig.root.join("missing").exists());
        }
    }

    #[test]
    fn folders_and_files_are_told_apart() {
        let mut rig = Rig::new();
        let none = CreateOptions::empty;
        let folder = || CreateOptions::FILE_DIRECTORY_FILE;
        let file = || CreateOptions::FILE_NON_DIRECTORY_FILE;
        let reply = rig.create("\\newdir", CreateDisposition::FILE_CREATE, folder());
        assert_eq!((status_of(&reply), reply[20]), (SUCCESS, FILE_CREATED));
        assert!(rig.root.join("newdir").is_dir());
        let cases = [
            ("\\sub", CreateDisposition::FILE_OPEN, file(), FILE_IS_A_DIRECTORY),
            ("\\sub", CreateDisposition::FILE_OVERWRITE_IF, none(), FILE_IS_A_DIRECTORY),
            ("\\sub", CreateDisposition::FILE_SUPERSEDE, none(), FILE_IS_A_DIRECTORY),
            ("\\sub", CreateDisposition::FILE_CREATE, folder(), COLLISION),
            ("\\readme.txt", CreateDisposition::FILE_OPEN, folder(), NOT_A_DIRECTORY),
            ("\\readme.txt\\x", CreateDisposition::FILE_OPEN_IF, none(), NO_SUCH_FILE),
            ("\\x.txt", CreateDisposition::from(7), none(), NOT_SUPPORTED),
        ];
        for (path, disposition, options, status) in cases {
            let reply = rig.create(path, disposition, options);
            assert_eq!(status_of(&reply), status, "{path} {disposition:?}");
        }
        assert_eq!(fs::read_to_string(rig.root.join("readme.txt")).unwrap(), "hello share");
        assert!(rig.root.join("sub").join("inner.txt").is_file());
        assert!(!rig.root.join("x.txt").exists());
    }

    #[test]
    fn a_file_and_an_empty_folder_marked_delete_on_close_go_with_the_close() {
        let mut rig = Rig::new();
        fs::create_dir(rig.root.join("empty")).unwrap();
        let delete = || CreateOptions::FILE_DELETE_ON_CLOSE;
        let file = rig.opened("\\readme.txt", delete());
        let folder = rig.opened("\\empty", delete() | CreateOptions::FILE_DIRECTORY_FILE);
        let full = rig.opened("\\sub", delete() | CreateOptions::FILE_DIRECTORY_FILE);
        assert!(rig.root.join("readme.txt").exists() && rig.root.join("empty").exists());
        assert_eq!(rig.close(file), SUCCESS);
        assert_eq!(rig.close(folder), SUCCESS);
        assert_eq!(rig.close(full), NOT_EMPTY);
        assert_eq!(names_in(&rig.root), ["notes.txt", "sub"]);
        assert_eq!(names_in(&rig.root.join("sub")), ["inner.txt"]);
    }

    #[test]
    fn deletes_still_pending_when_the_session_ends_are_done() {
        let mut rig = Rig::new();
        fs::create_dir(rig.root.join("gone")).unwrap();
        fs::write(rig.root.join("gone").join("x.txt"), "x").unwrap();
        let delete = || CreateOptions::FILE_DELETE_ON_CLOSE;
        // The folder can go only once the file in it has.
        rig.opened("\\gone", delete() | CreateOptions::FILE_DIRECTORY_FILE);
        rig.opened("\\gone\\x.txt", delete());
        let readme = rig.file("\\readme.txt");
        assert_eq!(rig.dispose(readme, true), SUCCESS);
        rig.file("\\notes.txt");
        drop(std::mem::replace(&mut rig.drive, Drive::new(rig.root.clone(), "wisp".to_owned())));
        assert_eq!(names_in(&rig.root), ["notes.txt", "sub"]);
    }

    #[test]
    fn reads_and_writes_land_at_their_offsets() {
        let mut rig = Rig::new();
        let id = rig.file("\\readme.txt");
        assert_eq!(rig.read(id, 6, 100), (SUCCESS, b"share".to_vec()));
        assert_eq!(rig.read(id, 0, 5), (SUCCESS, b"hello".to_vec()));
        assert_eq!(rig.read(id, 50, 10), (SUCCESS, Vec::new()));
        assert_eq!(rig.write(id, 6, b"SHARE"), (SUCCESS, 5));
        assert_eq!(rig.write(id, 13, b"!"), (SUCCESS, 1));
        assert_eq!(rig.read(id, 0, 100), (SUCCESS, b"hello SHARE\0\0!".to_vec()));
        assert_eq!(rig.close(id), SUCCESS);
        assert_eq!(fs::read(rig.root.join("readme.txt")).unwrap(), b"hello SHARE\0\0!");
        let folder = rig.folder("\\sub");
        assert_eq!(rig.read(folder, 0, 10), (FILE_IS_A_DIRECTORY, Vec::new()));
        assert_eq!(rig.write(folder, 0, b"x"), (FILE_IS_A_DIRECTORY, 0));
    }

    #[test]
    fn a_file_is_opened_for_writing_only_when_the_server_asks_to_write() {
        let mut rig = Rig::new();
        let read = DesiredAccess::GENERIC_READ;
        let open = CreateDisposition::FILE_OPEN;
        let reply = rig.create_for(read.clone(), "\\notes.txt", open, CreateOptions::empty());
        assert_eq!(status_of(&reply), SUCCESS);
        let id = u32_at(&reply, 16);
        assert_eq!(rig.read(id, 0, 10), (SUCCESS, b"notes".to_vec()));
        // A program here that lets others only read can still open the file, and the server too.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

            let mut local = OpenOptions::new();
            let notes = rig.root.join("notes.txt");
            let held = local.read(true).share_mode(FILE_SHARE_READ).open(notes);
            assert!(held.is_ok(), "{held:?}");
            let again = rig.create_for(read, "\\notes.txt", open, CreateOptions::empty());
            assert_eq!(status_of(&again), SUCCESS);
        }
        assert_eq!(rig.write(id, 0, b"x"), (ACCESS_DENIED, 0));
        let end = FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 });
        assert_eq!(rig.set(id, end), ACCESS_DENIED);
        assert_eq!(rig.close(id), SUCCESS);
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "notes");
        for write in [
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE,
            DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY,
            DesiredAccess::GENERIC_WRITE,
            DesiredAccess::GENERIC_ALL,
            DesiredAccess::MAXIMUM_ALLOWED,
        ] {
            let reply = rig.create_for(write.clone(), "\\notes.txt", open, CreateOptions::empty());
            let id = u32_at(&reply, 16);
            assert_eq!(rig.write(id, 5, b"!"), (SUCCESS, 1), "{write:?}");
            assert_eq!(rig.close(id), SUCCESS);
        }
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "notes!");
    }

    #[test]
    fn a_read_of_four_gigabytes_gets_one_mebibyte() {
        let mut rig = Rig::new();
        File::create(rig.root.join("big.bin")).unwrap().set_len(3 << 20).unwrap();
        let id = rig.file("\\big.bin");
        let (status, data) = rig.read(id, 0, u32::MAX);
        assert_eq!((status, data.len()), (SUCCESS, 1 << 20));
        let (status, data) = rig.read(id, (5 << 20) / 2, u32::MAX);
        assert_eq!((status, data.len()), (SUCCESS, 1 << 19));
    }

    #[test]
    fn file_information_basic_standard_and_attribute_tag() {
        let mut rig = Rig::new();
        let file = rig.file("\\readme.txt");
        let folder = rig.folder("\\sub");
        let basic = rig.query(file, 4);
        assert_eq!((status_of(&basic), u32_at(&basic, 16)), (SUCCESS, 36));
        assert!(i64_at(&basic, 20 + 16) > UNIX_IN_FILETIME, "a write time after 1970");
        assert_eq!(u32_at(&basic, 20 + 32) & (DIRECTORY | ARCHIVE), ARCHIVE);
        let standard = rig.query(file, 5);
        assert_eq!((status_of(&standard), u32_at(&standard, 16)), (SUCCESS, 22));
        assert_eq!(i64_at(&standard, 20 + 8), 11, "the end of file");
        assert_eq!((standard[20 + 20], standard[20 + 21]), (0, 0), "no delete, no folder");
        let tag = rig.query(file, 35);
        assert_eq!((status_of(&tag), u32_at(&tag, 16)), (SUCCESS, 8));
        assert_eq!(u32_at(&tag, 20) & (DIRECTORY | ARCHIVE), ARCHIVE);
        assert_eq!(u32_at(&tag, 24), 0, "no reparse tag");
        let basic = rig.query(folder, 4);
        assert_eq!(u32_at(&basic, 20 + 32) & (DIRECTORY | ARCHIVE), DIRECTORY);
        assert_eq!(rig.query(folder, 5)[20 + 21], 1, "a folder");
        for class in [34, 18, 6] {
            let reply = rig.query(file, class);
            assert_eq!((status_of(&reply), u32_at(&reply, 16)), (NOT_SUPPORTED, 0), "{class}");
        }
    }

    #[test]
    fn volume_information_volume_size_attribute_full_size_and_device() {
        let mut rig = Rig::new();
        let id = rig.folder("\\");
        let volume = rig.volume(id, 1);
        assert_eq!(status_of(&volume), SUCCESS);
        assert_eq!(u32_at(&volume, 20 + 12), 10, "the label's length with its terminator");
        assert_eq!(utf16(&volume[20 + 17..]), "wisp\0");
        let total = space(&rig.root).unwrap().total / 4096;
        let size = rig.volume(id, 3);
        assert_eq!(status_of(&size), SUCCESS);
        let (units, free) = (i64_at(&size, 20), i64_at(&size, 28));
        assert!(units > 0 && units as u64 == total, "{units} units, {total} measured");
        assert!((0..=units).contains(&free), "{free} free of {units}");
        assert_eq!((u32_at(&size, 36), u32_at(&size, 40)), (8, 512));
        let full = rig.volume(id, 7);
        assert_eq!(status_of(&full), SUCCESS);
        let (units, caller, actual) = (i64_at(&full, 20), i64_at(&full, 28), i64_at(&full, 36));
        assert_eq!(units as u64, total);
        assert!((0..=units).contains(&caller) && (0..=units).contains(&actual));
        assert_eq!((u32_at(&full, 44), u32_at(&full, 48)), (8, 512));
        let attribute = rig.volume(id, 5);
        assert_eq!(status_of(&attribute), SUCCESS);
        assert_eq!(utf16(&attribute[20 + 12..]), "NTFS\0");
        let device = rig.volume(id, 4);
        assert_eq!((status_of(&device), u32_at(&device, 20)), (SUCCESS, FILE_DEVICE_DISK));
        let label = rig.volume(id, 2);
        assert_eq!((status_of(&label), u32_at(&label, 16)), (NOT_SUPPORTED, 0));
    }

    #[test]
    fn listings_by_class_and_pattern_end_with_no_more_files() {
        let mut rig = Rig::new();
        let id = rig.folder("\\");
        for class in [1, 2, 3, 12] {
            let all = sorted(&["notes.txt", "readme.txt", "sub"]);
            assert_eq!(rig.list(id, class, "\\*"), Ok(all), "{class}");
            let text = sorted(&["notes.txt", "readme.txt"]);
            assert_eq!(rig.list(id, class, "\\*.txt"), Ok(text), "{class}");
            let one = || Ok(sorted(&["readme.txt"]));
            assert_eq!(rig.list(id, class, "\\?eadme.txt"), one(), "{class}");
            assert_eq!(rig.list(id, class, "\\README.TXT"), one(), "{class}");
            assert_eq!(rig.list(id, class, "/sub/*"), Ok(sorted(&["inner.txt"])), "{class}");
            assert_eq!(rig.list(id, class, "\\*.doc"), Err(NO_SUCH_FILE), "{class}");
            assert_eq!(rig.list(id, class, "\\missing\\*"), Err(NO_SUCH_FILE), "{class}");
            assert_eq!(rig.list(id, class, "\\..\\*"), Err(ACCESS_DENIED), "{class}");
        }
        let fresh = rig.folder("\\");
        let before = rig.directory(fresh, 3, false, "");
        assert_eq!(status_of(&before), NO_MORE_FILES, "no listing begun");
    }

    #[test]
    fn a_listing_carries_each_entry_s_size_and_kind() {
        let mut rig = Rig::new();
        let id = rig.folder("\\");
        let mut seen = Vec::new();
        let mut reply = rig.directory(id, 3, true, "\\*");
        while status_of(&reply) == SUCCESS {
            let entry = &reply[20..];
            let kind = u32_at(entry, 56) & (DIRECTORY | ARCHIVE);
            seen.push((entry_name(3, entry), kind, i64_at(entry, 40)));
            reply = rig.directory(id, 3, false, "");
        }
        seen.sort();
        let sub = seen.iter().find(|(name, ..)| name == "sub").unwrap().1;
        assert_eq!(sub, DIRECTORY);
        let files: Vec<_> = seen.iter().filter(|(name, ..)| name != "sub").cloned().collect();
        let expected =
            [("notes.txt".to_owned(), ARCHIVE, 5), ("readme.txt".to_owned(), ARCHIVE, 11)];
        assert_eq!(files, expected);
    }

    #[test]
    fn a_listing_shows_a_link_and_not_what_it_points_to() {
        let mut rig = Rig::new();
        let outside = rig.outside();
        rig.link_folder("out", &outside);
        let id = rig.folder("\\");
        let reply = rig.directory(id, 1, true, "\\out");
        assert_eq!(status_of(&reply), SUCCESS);
        let entry = &reply[20..];
        assert_eq!(entry_name(1, entry), "out");
        assert_eq!(u32_at(entry, 56) & DIRECTORY, 0, "the link is not shown as the folder");
        if !rig.link_file("secret", &outside.join("secret.txt")) {
            return;
        }
        let reply = rig.directory(id, 1, true, "\\secret");
        assert_eq!(status_of(&reply), SUCCESS);
        let link = fs::symlink_metadata(rig.root.join("secret")).unwrap().len() as i64;
        assert_ne!(link, 6);
        assert_eq!(i64_at(&reply[20..], 40), link, "the link's own size, not the file's");
    }

    #[test]
    fn set_basic_end_of_file_and_allocation() {
        let mut rig = Rig::new();
        let id = rig.file("\\readme.txt");
        let length = |rig: &Rig| fs::metadata(rig.root.join("readme.txt")).unwrap().len();
        let basic = FileInformationClass::Basic(FileBasicInformation {
            creation_time: 0,
            last_access_time: 0,
            last_write_time: 0,
            change_time: 0,
            file_attributes: FileAttributes::FILE_ATTRIBUTE_ARCHIVE,
        });
        assert_eq!(rig.set(id, basic), SUCCESS);
        let end = |end_of_file| {
            FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file })
        };
        let allocation = |allocation_size| {
            FileInformationClass::Allocation(FileAllocationInformation { allocation_size })
        };
        assert_eq!((rig.set(id, end(5)), length(&rig)), (SUCCESS, 5));
        assert_eq!((rig.set(id, end(20)), length(&rig)), (SUCCESS, 20));
        assert_eq!((rig.set(id, allocation(100)), length(&rig)), (SUCCESS, 20));
        assert_eq!((rig.set(id, allocation(3)), length(&rig)), (SUCCESS, 3));
        assert_eq!((rig.set(id, end(-1)), length(&rig)), (UNSUCCESSFUL, 3));
        assert_eq!(fs::read(rig.root.join("readme.txt")).unwrap(), b"hel");
        let folder = rig.folder("\\sub");
        assert_eq!(rig.set(folder, end(0)), FILE_IS_A_DIRECTORY);
        assert_eq!(rig.set(folder, allocation(0)), FILE_IS_A_DIRECTORY);
    }

    #[test]
    fn a_disposition_deletes_at_the_close_and_never_a_full_folder() {
        let mut rig = Rig::new();
        let file = rig.file("\\readme.txt");
        assert_eq!(rig.dispose(file, true), SUCCESS);
        assert_eq!(rig.query(file, 5)[20 + 20], 1, "the delete is pending");
        assert!(rig.root.join("readme.txt").exists());
        assert_eq!(rig.close(file), SUCCESS);
        assert!(!rig.root.join("readme.txt").exists());
        let kept = rig.file("\\notes.txt");
        assert_eq!(rig.dispose(kept, true), SUCCESS);
        assert_eq!(rig.dispose(kept, false), SUCCESS);
        assert_eq!(rig.close(kept), SUCCESS);
        assert!(rig.root.join("notes.txt").exists());
        let full = rig.folder("\\sub");
        assert_eq!(rig.dispose(full, true), NOT_EMPTY);
        assert_eq!(rig.close(full), SUCCESS);
        assert!(rig.root.join("sub").join("inner.txt").exists());
        fs::remove_file(rig.root.join("sub").join("inner.txt")).unwrap();
        let empty = rig.folder("\\sub");
        assert_eq!(rig.dispose(empty, true), SUCCESS);
        assert_eq!(rig.close(empty), SUCCESS);
        assert_eq!(names_in(&rig.root), ["notes.txt"]);
    }

    #[test]
    fn renames_stay_inside_the_share() {
        let mut rig = Rig::new();
        let id = rig.file("\\readme.txt");
        assert_eq!(rig.rename(id, "\\sub\\moved.txt", false), SUCCESS);
        assert!(!rig.root.join("readme.txt").exists());
        let moved = rig.root.join("sub").join("moved.txt");
        assert_eq!(fs::read_to_string(&moved).unwrap(), "hello share");
        assert_eq!(rig.read(id, 0, 5), (SUCCESS, b"hello".to_vec()), "the handle follows the file");
        assert_eq!(rig.rename(id, "\\notes.txt", false), COLLISION);
        assert_eq!(fs::read_to_string(&moved).unwrap(), "hello share");
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "notes");
        assert_eq!(rig.rename(id, "\\notes.txt", true), SUCCESS);
        assert!(!moved.exists());
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "hello share");
        for to in ["..\\outside\\x.txt", "\\..\\outside\\secret.txt", "/../outside/x", "C:\\x"] {
            assert_eq!(rig.rename(id, to, true), ACCESS_DENIED, "{to}");
        }
        assert_eq!(rig.rename(id, "\\missing\\x.txt", false), NO_SUCH_FILE);
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "hello share");
        assert_eq!(names_in(&rig.outside()), ["secret.txt"]);
        assert_eq!(fs::read_to_string(rig.outside().join("secret.txt")).unwrap(), "secret");
        let folder = rig.folder("\\sub");
        assert_eq!(rig.rename(folder, "\\renamed", false), SUCCESS);
        assert_eq!(names_in(&rig.root.join("renamed")), ["inner.txt"]);
        assert_eq!(rig.close(id), SUCCESS);
    }

    #[test]
    fn handles_inside_a_renamed_folder_follow_it() {
        let mut rig = Rig::new();
        fs::create_dir_all(rig.root.join("a").join("inner")).unwrap();
        fs::write(rig.root.join("a").join("x.txt"), "old").unwrap();
        let file = rig.file("\\a\\x.txt");
        assert_eq!(rig.dispose(file, true), SUCCESS);
        let folder = rig.folder("\\a");
        // Windows renames no folder that holds an open file.
        let renamed = rig.rename(folder, "\\b", false);
        assert_eq!(renamed, if cfg!(windows) { ACCESS_DENIED } else { SUCCESS });
        if cfg!(windows) {
            assert_eq!(rig.close(file), SUCCESS);
            assert_eq!(names_in(&rig.root.join("a")), ["inner"]);
            fs::write(rig.root.join("a").join("x.txt"), "old").unwrap();
        } else {
            fs::create_dir(rig.root.join("a")).unwrap();
            fs::write(rig.root.join("a").join("x.txt"), "new").unwrap();
            assert_eq!(rig.close(file), SUCCESS);
            assert_eq!(names_in(&rig.root.join("a")), ["x.txt"]);
            assert_eq!(names_in(&rig.root.join("b")), ["inner"]);
            fs::remove_dir_all(rig.root.join("a")).unwrap();
            fs::rename(rig.root.join("b"), rig.root.join("a")).unwrap();
            fs::write(rig.root.join("a").join("x.txt"), "old").unwrap();
        }
        assert_eq!(rig.close(folder), SUCCESS);
        // A folder holds no open file here, so Windows renames the folder it is in too.
        let inner = rig.folder("\\a\\inner");
        assert_eq!(rig.dispose(inner, true), SUCCESS);
        let parent = rig.folder("\\a");
        assert_eq!(rig.rename(parent, "\\b", false), SUCCESS);
        fs::create_dir_all(rig.root.join("a").join("inner")).unwrap();
        assert_eq!(rig.close(inner), SUCCESS);
        assert_eq!(names_in(&rig.root.join("a")), ["inner"]);
        assert_eq!(names_in(&rig.root.join("b")), ["x.txt"]);
        assert_eq!(rig.dispose(parent, true), NOT_EMPTY);
        assert_eq!(rig.close(parent), SUCCESS);
    }

    // NTFS refuses a rename with replace onto a file another handle holds, with or without a delete
    // pending on it (measured 2026-11-06).
    #[test]
    fn a_rename_onto_a_file_another_handle_holds_is_refused() {
        for pending in [false, true] {
            let mut rig = Rig::new();
            fs::write(rig.root.join("a.txt"), "moved").unwrap();
            fs::write(rig.root.join("b.txt"), "old b").unwrap();
            let held = rig.file("\\b.txt");
            if pending {
                assert_eq!(rig.dispose(held, true), SUCCESS);
            }
            let moving = rig.file("\\a.txt");
            assert_eq!(rig.rename(moving, "\\b.txt", true), ACCESS_DENIED, "pending: {pending}");
            assert_eq!(fs::read_to_string(rig.root.join("a.txt")).unwrap(), "moved");
            assert_eq!(fs::read_to_string(rig.root.join("b.txt")).unwrap(), "old b");
            assert_eq!(rig.close(moving), SUCCESS);
            assert_eq!(rig.close(held), SUCCESS);
            assert_eq!(fs::read_to_string(rig.root.join("a.txt")).unwrap(), "moved");
            assert_eq!(rig.root.join("b.txt").exists(), !pending, "pending: {pending}");
        }
        // Once the other handle is closed the rename goes through.
        let mut rig = Rig::new();
        fs::write(rig.root.join("a.txt"), "moved").unwrap();
        let held = rig.file("\\notes.txt");
        assert_eq!(rig.close(held), SUCCESS);
        let moving = rig.file("\\a.txt");
        assert_eq!(rig.rename(moving, "\\notes.txt", true), SUCCESS);
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "moved");
    }

    #[test]
    fn a_handle_follows_a_rename_through_another_handle() {
        let mut rig = Rig::new();
        let pending = rig.file("\\notes.txt");
        assert_eq!(rig.dispose(pending, true), SUCCESS);
        let other = rig.file("\\notes.txt");
        assert_eq!(rig.rename(other, "\\moved.txt", false), SUCCESS);
        fs::write(rig.root.join("notes.txt"), "new").unwrap();
        assert_eq!(rig.close(other), SUCCESS);
        assert_eq!(rig.close(pending), SUCCESS);
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "new");
        assert!(!rig.root.join("moved.txt").exists());
    }

    #[test]
    fn a_rename_that_changes_only_the_case_is_no_collision() {
        let mut rig = Rig::new();
        let id = rig.file("\\readme.txt");
        assert_eq!(rig.rename(id, "\\README.TXT", false), SUCCESS);
        assert_eq!(names_in(&rig.root), ["README.TXT", "notes.txt", "sub"]);
    }

    #[test]
    fn the_share_root_is_neither_deleted_nor_renamed() {
        let mut rig = Rig::new();
        for name in ["readme.txt", "notes.txt"] {
            fs::remove_file(rig.root.join(name)).unwrap();
        }
        fs::remove_dir_all(rig.root.join("sub")).unwrap();
        let delete = CreateOptions::FILE_DIRECTORY_FILE | CreateOptions::FILE_DELETE_ON_CLOSE;
        let reply = rig.create("\\", CreateDisposition::FILE_OPEN, delete);
        assert_eq!(status_of(&reply), ACCESS_DENIED);
        let id = rig.folder("\\");
        assert_eq!(rig.dispose(id, true), ACCESS_DENIED);
        assert_eq!(rig.rename(id, "\\inside", true), ACCESS_DENIED);
        assert_eq!(rig.close(id), SUCCESS);
        assert!(rig.root.is_dir());
        assert_eq!(names_in(&rig.root), Vec::<String>::new());
    }

    #[test]
    fn a_link_out_of_the_share_is_neither_followed_nor_replaced() {
        let mut rig = Rig::new();
        let outside = rig.outside();
        rig.link_folder("out", &outside);
        rig.link_folder("dangling", &outside.join("planted"));
        let file = CreateOptions::empty;
        let folder = || CreateOptions::FILE_DIRECTORY_FILE;
        let cases = [
            ("\\out", CreateDisposition::FILE_OPEN, folder()),
            ("\\out\\secret.txt", CreateDisposition::FILE_OPEN, file()),
            ("\\out\\secret.txt", CreateDisposition::FILE_OVERWRITE_IF, file()),
            ("\\out\\new.txt", CreateDisposition::FILE_OPEN_IF, file()),
            ("\\out\\newdir", CreateDisposition::FILE_CREATE, folder()),
            ("\\out", CreateDisposition::FILE_SUPERSEDE, file()),
            ("\\out", CreateDisposition::FILE_OVERWRITE_IF, file()),
            ("\\dangling", CreateDisposition::FILE_OPEN_IF, file()),
            ("\\dangling", CreateDisposition::FILE_SUPERSEDE, file()),
            ("\\dangling", CreateDisposition::FILE_CREATE, file()),
            ("\\dangling", CreateDisposition::FILE_CREATE, folder()),
            ("\\dangling\\new.txt", CreateDisposition::FILE_OPEN_IF, file()),
        ];
        for (path, disposition, options) in cases {
            let reply = rig.create(path, disposition, options);
            assert_eq!(status_of(&reply), ACCESS_DENIED, "{path} {disposition:?}");
        }
        let id = rig.folder("\\");
        assert_eq!(rig.list(id, 3, "\\out\\*"), Err(ACCESS_DENIED));
        let readme = rig.file("\\readme.txt");
        assert_eq!(rig.rename(readme, "\\out\\stolen.txt", true), ACCESS_DENIED);
        assert_eq!(rig.rename(readme, "\\dangling\\stolen.txt", true), ACCESS_DENIED);
        assert_eq!(names_in(&outside), ["secret.txt"]);
        assert_eq!(fs::read_to_string(outside.join("secret.txt")).unwrap(), "secret");
        assert_eq!(fs::read_to_string(rig.root.join("readme.txt")).unwrap(), "hello share");
    }

    // A link to `notes.txt` inside the share, `link.txt`, or None where none can be made.
    fn file_link(rig: &Rig) -> Option<PathBuf> {
        rig.link_file("link.txt", &rig.root.join("notes.txt")).then(|| rig.root.join("link.txt"))
    }

    #[test]
    fn a_link_inside_the_share_is_read_and_written_through() {
        let mut rig = Rig::new();
        let Some(_) = file_link(&rig) else { return };
        let id = rig.file("\\link.txt");
        assert_eq!(rig.read(id, 0, 100), (SUCCESS, b"notes".to_vec()));
        assert_eq!(rig.write(id, 5, b"!"), (SUCCESS, 1));
        assert_eq!(rig.close(id), SUCCESS);
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "notes!");
    }

    #[test]
    fn deleting_a_link_deletes_the_link_and_not_its_target() {
        for disposition in [false, true] {
            let mut rig = Rig::new();
            let Some(link) = file_link(&rig) else { return };
            let id = if disposition {
                let id = rig.file("\\link.txt");
                assert_eq!(rig.dispose(id, true), SUCCESS);
                id
            } else {
                rig.opened("\\link.txt", CreateOptions::FILE_DELETE_ON_CLOSE)
            };
            assert_eq!(rig.close(id), SUCCESS, "disposition: {disposition}");
            assert!(fs::symlink_metadata(&link).is_err(), "disposition: {disposition}");
            let notes = fs::read_to_string(rig.root.join("notes.txt"));
            assert_eq!(notes.unwrap(), "notes", "disposition: {disposition}");
        }
    }

    #[test]
    fn renaming_a_link_renames_the_link_and_not_its_target() {
        let mut rig = Rig::new();
        let Some(link) = file_link(&rig) else { return };
        let id = rig.file("\\link.txt");
        assert_eq!(rig.rename(id, "\\notes.txt", false), COLLISION);
        assert_eq!(rig.rename(id, "\\sub\\moved.txt", false), SUCCESS);
        assert!(fs::symlink_metadata(&link).is_err());
        let moved = rig.root.join("sub").join("moved.txt");
        assert!(fs::symlink_metadata(&moved).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "notes");
        assert_eq!(rig.read(id, 0, 100), (SUCCESS, b"notes".to_vec()));
        assert_eq!(rig.close(id), SUCCESS);
        assert_eq!(names_in(&rig.root), ["notes.txt", "readme.txt", "sub"]);
    }

    #[test]
    fn deleting_or_renaming_a_link_to_a_folder_acts_on_the_link() {
        for disposition in [false, true] {
            let mut rig = Rig::new();
            rig.link_folder("inner", &rig.root.join("sub"));
            let id = if disposition {
                let id = rig.folder("\\inner");
                assert_eq!(rig.dispose(id, true), SUCCESS, "a folder link holds nothing itself");
                id
            } else {
                let delete = CreateOptions::FILE_DELETE_ON_CLOSE;
                rig.opened("\\inner", delete | CreateOptions::FILE_DIRECTORY_FILE)
            };
            assert_eq!(rig.close(id), SUCCESS, "disposition: {disposition}");
            assert_eq!(names_in(&rig.root), ["notes.txt", "readme.txt", "sub"]);
            assert_eq!(names_in(&rig.root.join("sub")), ["inner.txt"]);
        }
        let mut rig = Rig::new();
        rig.link_folder("inner", &rig.root.join("sub"));
        let id = rig.folder("\\inner");
        assert_eq!(rig.rename(id, "\\renamed", false), SUCCESS);
        assert_eq!(names_in(&rig.root), ["notes.txt", "readme.txt", "renamed", "sub"]);
        assert_eq!(names_in(&rig.root.join("renamed")), ["inner.txt"]);
        assert_eq!(names_in(&rig.root.join("sub")), ["inner.txt"]);
        assert_eq!(rig.close(id), SUCCESS);
    }

    // An 8.3 name is a spelling of the same entry that matches its name in no case.
    #[cfg(windows)]
    #[test]
    fn a_link_opens_under_its_short_name() {
        use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

        let mut rig = Rig::new();
        rig.link_folder("LongLinkName", &rig.root.join("sub"));
        let long = rig.root.join("LongLinkName");
        let wide: Vec<u16> = long.as_os_str().encode_wide().chain([0]).collect();
        let mut short = vec![0u16; 1024];
        let length = unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), 1024) };
        short.truncate(length as usize);
        let short = PathBuf::from(std::ffi::OsString::from_wide(&short));
        let name = short.file_name().map(|name| name.to_string_lossy().into_owned());
        let Some(name) = name.filter(|name| name != "LongLinkName") else {
            println!("no 8.3 name on this volume ({short:?}), test skipped");
            return;
        };
        let id = rig.folder(&format!("\\{name}"));
        assert_eq!(rig.list(id, 12, "\\sub\\*"), Ok(sorted(&["inner.txt"])));
        assert_eq!(rig.dispose(id, true), SUCCESS);
        assert_eq!(rig.close(id), SUCCESS);
        assert_eq!(names_in(&rig.root), ["notes.txt", "readme.txt", "sub"]);
        assert_eq!(names_in(&rig.root.join("sub")), ["inner.txt"]);
    }

    // The folder cannot be listed, so the link's name cannot be read.
    #[test]
    fn an_open_through_a_link_that_fails_leaves_the_file_as_it_was() {
        let mut rig = Rig::new();
        let Some(_) = file_link(&rig) else { return };
        #[cfg(windows)]
        let (user, root) = (std::env::var("USERNAME").unwrap(), rig.root.clone());
        #[cfg(windows)]
        let icacls = |args: &[&str]| {
            let ran = std::process::Command::new("icacls").arg(&root).args(args).output();
            assert!(ran.unwrap().status.success(), "icacls {args:?}");
        };
        #[cfg(windows)]
        icacls(&["/deny", &format!("{user}:(RD)")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&rig.root, fs::Permissions::from_mode(0o300)).unwrap();
            if fs::read_dir(&rig.root).is_ok() {
                fs::set_permissions(&rig.root, fs::Permissions::from_mode(0o700)).unwrap();
                println!("the folder can still be listed (root ignores its mode), test skipped");
                return;
            }
        }
        let overwrite = CreateDisposition::FILE_OVERWRITE;
        let reply = rig.create("\\link.txt", overwrite, CreateOptions::FILE_NON_DIRECTORY_FILE);
        #[cfg(windows)]
        icacls(&["/remove:d", &user]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&rig.root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(status_of(&reply), ACCESS_DENIED);
        assert_eq!(fs::read_to_string(rig.root.join("notes.txt")).unwrap(), "notes");
    }

    // Windows takes any case of a name, so two handles on one link can spell it differently.
    #[cfg(windows)]
    #[test]
    fn a_rename_of_a_link_reaches_a_handle_that_spelled_it_otherwise() {
        let mut rig = Rig::new();
        rig.link_folder("Link", &rig.root.join("sub"));
        let pending = rig.folder("\\Link");
        assert_eq!(rig.dispose(pending, true), SUCCESS);
        let renaming = rig.folder("\\link");
        assert_eq!(rig.rename(renaming, "\\other", false), SUCCESS);
        assert_eq!(rig.close(renaming), SUCCESS);
        fs::create_dir(rig.root.join("Link")).unwrap();
        assert_eq!(rig.close(pending), SUCCESS);
        assert_eq!(names_in(&rig.root), ["Link", "notes.txt", "readme.txt", "sub"]);
        assert_eq!(names_in(&rig.root.join("sub")), ["inner.txt"]);
    }

    #[test]
    fn a_dangling_file_link_out_of_the_share_creates_nothing_outside() {
        let mut rig = Rig::new();
        let planted = rig.outside().join("planted.txt");
        if !rig.link_file("dangling", &planted) {
            return;
        }
        for disposition in [
            CreateDisposition::FILE_SUPERSEDE,
            CreateDisposition::FILE_CREATE,
            CreateDisposition::FILE_OPEN_IF,
            CreateDisposition::FILE_OVERWRITE_IF,
        ] {
            let reply = rig.create("\\dangling", disposition, CreateOptions::empty());
            assert_eq!(status_of(&reply), ACCESS_DENIED, "{disposition:?}");
            assert!(!planted.exists(), "{disposition:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_handle_cap_is_a_quarter_of_the_descriptor_limit_and_at_most_1024() {
        let caps = [
            (0, 0),
            (3, 0),
            (256, 64),
            (1024, 256),
            (1027, 256),
            (4096, 1024),
            (1 << 20, 1024),
            (libc::RLIM_INFINITY, 1024),
        ];
        for (descriptors, cap) in caps {
            assert_eq!(handle_cap(descriptors), cap, "{descriptors}");
        }
    }

    #[test]
    fn opens_past_the_handle_cap_are_refused() {
        let mut rig = Rig::new();
        let cap = rig.drive.max_open;
        #[cfg(windows)]
        assert_eq!(cap, 1024);
        #[cfg(unix)]
        {
            let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) }, 0);
            assert_eq!(cap as u64, (limit.rlim_cur / 4).min(1024), "soft limit {}", limit.rlim_cur);
        }
        let ids: Vec<u32> = (0..cap).map(|_| rig.folder("\\")).collect();
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), cap);
        assert!(!ids.contains(&0));
        let folder = CreateOptions::FILE_DIRECTORY_FILE;
        let reply = rig.create("\\", CreateDisposition::FILE_OPEN, folder);
        assert_eq!(status_of(&reply), TOO_MANY_OPENED_FILES);
        let reply = rig.create("\\new.txt", CreateDisposition::FILE_CREATE, CreateOptions::empty());
        assert_eq!(status_of(&reply), TOO_MANY_OPENED_FILES);
        assert!(!rig.root.join("new.txt").exists());
        assert_eq!(rig.close(ids[0]), SUCCESS);
        let id = rig.folder("\\");
        assert!(!ids[1..].contains(&id));
    }

    #[test]
    fn requests_on_a_closed_or_unknown_handle_are_refused() {
        let mut rig = Rig::new();
        let closed = rig.file("\\readme.txt");
        assert_eq!(rig.close(closed), SUCCESS);
        for id in [closed, 9999] {
            assert_eq!(rig.close(id), INVALID_HANDLE);
            assert_eq!(rig.read(id, 0, 10), (INVALID_HANDLE, Vec::new()));
            assert_eq!(rig.write(id, 0, b"x"), (INVALID_HANDLE, 0));
            assert_eq!(status_of(&rig.query(id, 4)), INVALID_HANDLE);
            assert_eq!(status_of(&rig.volume(id, 1)), INVALID_HANDLE);
            assert_eq!(status_of(&rig.directory(id, 3, true, "\\*")), INVALID_HANDLE);
            assert_eq!(status_of(&rig.directory(id, 3, false, "")), INVALID_HANDLE);
            let end = FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 });
            assert_eq!(rig.set(id, end), INVALID_HANDLE);
            assert_eq!(rig.dispose(id, true), INVALID_HANDLE);
            assert_eq!(rig.rename(id, "\\x.txt", true), INVALID_HANDLE);
            let device_io_request = rig.io(id, MajorFunction::LockControl, 0);
            let lock = ServerDriveLockControlRequest { device_io_request };
            let reply = rig.send(ServerDriveIoRequest::ServerDriveLockControlRequest(lock));
            assert_eq!(status_of(&reply), INVALID_HANDLE);
            let header = rig.io(id, MajorFunction::DeviceControl, 0);
            let control = DeviceControlRequest {
                header,
                output_buffer_length: 0,
                input_buffer_length: 0,
                io_control_code: AnyIoCtlCode(0x0009_00A8),
            };
            let reply = rig.send(ServerDriveIoRequest::DeviceControlRequest(control));
            assert_eq!(status_of(&reply), INVALID_HANDLE);
            let device_io_request = rig.io(id, MajorFunction::DirectoryControl, 2);
            let notify = ServerDriveNotifyChangeDirectoryRequest {
                device_io_request,
                watch_tree: 0,
                completion_filter: 0x15,
            };
            let notify = ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(notify);
            assert_eq!(status_of(&rig.send(notify)), INVALID_HANDLE);
        }
        assert_eq!(names_in(&rig.root), ["notes.txt", "readme.txt", "sub"]);
        assert_eq!(fs::read_to_string(rig.root.join("readme.txt")).unwrap(), "hello share");
    }

    #[test]
    fn locks_and_device_controls_are_not_supported_and_change_notifications_wait() {
        let mut rig = Rig::new();
        let id = rig.folder("\\");
        let device_io_request = rig.io(id, MajorFunction::LockControl, 0);
        let lock = ServerDriveLockControlRequest { device_io_request };
        let reply = rig.send(ServerDriveIoRequest::ServerDriveLockControlRequest(lock));
        assert_eq!(status_of(&reply), NOT_SUPPORTED);
        let header = rig.io(id, MajorFunction::DeviceControl, 0);
        let control = DeviceControlRequest {
            header,
            output_buffer_length: 64,
            input_buffer_length: 0,
            io_control_code: AnyIoCtlCode(0x0009_00A8),
        };
        let reply = rig.send(ServerDriveIoRequest::DeviceControlRequest(control));
        assert_eq!((status_of(&reply), u32_at(&reply, 16)), (NOT_SUPPORTED, 0));
        let device_io_request = rig.io(id, MajorFunction::DirectoryControl, 2);
        let notify = ServerDriveNotifyChangeDirectoryRequest {
            device_io_request,
            watch_tree: 0,
            completion_filter: 0x15,
        };
        let notify = ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(notify);
        assert!(rig.replies(notify).is_empty(), "a change notification was answered");
        assert_eq!(rig.close(id), SUCCESS);
    }

    #[test]
    fn the_share_deleted_under_the_session_is_not_found() {
        let mut rig = Rig::new();
        let folder = rig.folder("\\");
        fs::remove_dir_all(&rig.root).unwrap();
        let cases = [
            ("\\", CreateDisposition::FILE_OPEN),
            ("\\readme.txt", CreateDisposition::FILE_OPEN),
            ("\\new.txt", CreateDisposition::FILE_OPEN_IF),
            ("\\sub\\inner.txt", CreateDisposition::FILE_OPEN),
            ("\\sub\\new", CreateDisposition::FILE_CREATE),
        ];
        for (path, disposition) in cases {
            let reply = rig.create(path, disposition, CreateOptions::empty());
            assert_eq!(status_of(&reply), NO_SUCH_FILE, "{path}");
        }
        assert!(!rig.root.exists());
        assert_eq!(status_of(&rig.query(folder, 4)), NO_SUCH_FILE);
        assert_eq!(status_of(&rig.directory(folder, 3, true, "\\*")), NO_SUCH_FILE);
        for class in [1, 3, 7] {
            assert_eq!(status_of(&rig.volume(folder, class)), NO_SUCH_FILE, "{class}");
        }
        assert_eq!(rig.close(folder), SUCCESS);
    }

    #[test]
    fn a_file_removed_while_open_is_still_answered() {
        let mut rig = Rig::new();
        let id = rig.file("\\readme.txt");
        fs::remove_file(rig.root.join("readme.txt")).unwrap();
        let mapped = [SUCCESS, NO_SUCH_FILE, ACCESS_DENIED, UNSUCCESSFUL];
        let (read, _) = rig.read(id, 0, 100);
        let (written, _) = rig.write(id, 0, b"x");
        let query = status_of(&rig.query(id, 5));
        for status in [read, written, query] {
            assert!(mapped.contains(&status), "{status:#010x}");
        }
        assert_eq!(rig.close(id), SUCCESS);
        assert!(!rig.root.join("readme.txt").exists());
    }

    #[test]
    fn system_errors_map_to_their_status() {
        let kinds = [
            (ErrorKind::NotFound, NO_SUCH_FILE),
            (ErrorKind::AlreadyExists, COLLISION),
            (ErrorKind::NotADirectory, NOT_A_DIRECTORY),
            (ErrorKind::IsADirectory, FILE_IS_A_DIRECTORY),
            (ErrorKind::PermissionDenied, ACCESS_DENIED),
            (ErrorKind::ReadOnlyFilesystem, ACCESS_DENIED),
            (ErrorKind::StorageFull, DISK_FULL),
            (ErrorKind::QuotaExceeded, DISK_FULL),
            (ErrorKind::DirectoryNotEmpty, NOT_EMPTY),
            (ErrorKind::InvalidInput, UNSUCCESSFUL),
            (ErrorKind::Other, UNSUCCESSFUL),
        ];
        for (kind, expected) in kinds {
            assert_eq!(u32::from(status(&io::Error::from(kind))), expected, "{kind:?}");
        }
        // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL, ERROR_ACCESS_DENIED, ERROR_DIR_NOT_EMPTY
        #[cfg(windows)]
        let codes = [(112, DISK_FULL), (39, DISK_FULL), (5, ACCESS_DENIED), (145, NOT_EMPTY)];
        #[cfg(unix)]
        let codes = [
            (libc::ENOSPC, DISK_FULL),
            (libc::EDQUOT, DISK_FULL),
            (libc::EACCES, ACCESS_DENIED),
            (libc::ENOTEMPTY, NOT_EMPTY),
            (libc::EMFILE, TOO_MANY_OPENED_FILES),
            (libc::ENFILE, TOO_MANY_OPENED_FILES),
        ];
        for (code, expected) in codes {
            let error = io::Error::from_raw_os_error(code);
            assert_eq!(u32::from(status(&error)), expected, "{error}");
        }
    }

    #[test]
    fn patterns_match_without_regard_to_ascii_case() {
        let matching = [
            ("*", "readme.txt"),
            ("*.txt", "readme.TXT"),
            ("?eadme.txt", "readme.txt"),
            ("README.TXT", "readme.txt"),
            ("r*e*.t?t", "readme.txt"),
            ("*e*", "readme.txt"),
            ("**", ""),
            ("", ""),
        ];
        for (pattern, name) in matching {
            assert!(matches(pattern, name), "{pattern:?} {name:?}");
        }
        let failing = [("*.doc", "readme.txt"), ("?", "ab"), ("readme", "readme.txt"), ("", "a")];
        for (pattern, name) in failing {
            assert!(!matches(pattern, name), "{pattern:?} {name:?}");
        }
        let (name, pattern) = ("a".repeat(255), "*a".repeat(100) + "b");
        let started = Instant::now();
        assert!(!matches(&pattern, &name));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    // What `cmd /c dir /b /a-d <pattern>` lists of these files in a local NTFS folder on Windows
    // Server 2025, measured with their 8.3 names stripped (2026-11-06).
    #[test]
    fn patterns_match_as_a_local_windows_folder_does() {
        let names = [
            ".env", "a", "a.b.c", "a.c", "a.txt", "ab", "ab.txt", "abc", "abc.t", "abc.tat",
            "abc.txt", "abd.txt", "axc.txt", "longfilename.txt", "noext", "x.y.txt",
        ];
        let all = &names[..];
        let listed: [(&str, &[&str]); 35] = [
            ("*.", &["a", "ab", "abc", "noext"]),
            ("abc.", &["abc"]),
            ("a.b.", &[]),
            ("a??.txt", &["a.txt", "ab.txt", "abc.txt", "abd.txt", "axc.txt"]),
            ("?.txt", &["a.txt"]),
            ("*.t?t", &[
                "a.txt", "ab.txt", "abc.tat", "abc.txt", "abd.txt", "axc.txt", "longfilename.txt",
                "x.y.txt",
            ]),
            ("*c.txt", &["abc.txt", "axc.txt"]),
            ("a*", &[
                "a", "a.b.c", "a.c", "a.txt", "ab", "ab.txt", "abc", "abc.t", "abc.tat", "abc.txt",
                "abd.txt", "axc.txt",
            ]),
            ("*", all),
            ("*.*", all),
            ("a?c", &["abc"]),
            ("??.txt", &["a.txt", "ab.txt"]),
            ("a.*", &["a", "a.b.c", "a.c", "a.txt"]),
            ("*.c", &["a.b.c", "a.c"]),
            ("a.b.?", &["a.b.c"]),
            ("abc.*", &["abc", "abc.t", "abc.tat", "abc.txt"]),
            ("*.txt", &[
                "a.txt", "ab.txt", "abc.txt", "abd.txt", "axc.txt", "longfilename.txt", "x.y.txt",
            ]),
            (".*", &[".env"]),
            ("*.?", &["a", "a.b.c", "a.c", "ab", "abc", "abc.t", "noext"]),
            ("?", &["a"]),
            ("???", &["a", "ab", "abc"]),
            ("abc.?", &["abc", "abc.t"]),
            ("a?", &["a", "ab"]),
            ("*b*", &["a.b.c", "ab", "ab.txt", "abc", "abc.t", "abc.tat", "abc.txt", "abd.txt"]),
            ("*.*.*", all),
            ("a.b.c.", &["a.b.c"]),
            ("*t", &[
                "a.txt", "ab.txt", "abc.t", "abc.tat", "abc.txt", "abd.txt", "axc.txt",
                "longfilename.txt", "noext", "x.y.txt",
            ]),
            ("?bc*", &["abc", "abc.t", "abc.tat", "abc.txt"]),
            ("a??", &["a", "ab", "abc"]),
            ("abc.t??", &["abc.t", "abc.tat", "abc.txt"]),
            ("*.t", &["abc.t"]),
            ("a.?", &["a", "a.c"]),
            ("??", &["a", "ab"]),
            ("x.*.txt", &["x.y.txt"]),
            ("*.e*", &[".env"]),
        ];
        // Where 8.3 names exist, `.env` gets ENV~1, and `*.` and `*.?` list it for that name.
        let left_out = [("*.", ".env"), ("*.?", ".env")];
        for (pattern, wanted) in listed {
            let names = names.into_iter().filter(|name| !left_out.contains(&(pattern, *name)));
            let names: Vec<&str> = names.collect();
            for asked in [pattern.to_owned(), pattern.to_ascii_uppercase()] {
                let found: Vec<&str> =
                    names.iter().copied().filter(|name| matches(&asked, name)).collect();
                assert_eq!(found, wanted, "{asked:?}");
            }
        }
        // Names that end in a dot, which only an NT path makes: what `dir` lists of them in a
        // local NTFS folder on Windows 11, where their 8.3 names match none of these patterns, and
        // RtlIsNameInExpression says of the pattern FindFirstFile makes (2026-11-06).
        let dotted = [
            ("a?", "a.", true),
            ("a?", "a..", false),
            ("??", "a.", true),
            ("a??", "a.", true),
            ("a??", "ab.", true),
            ("a??", "a..", false),
            ("a.?", "a.", true),
            ("a.?", "a..", true),
        ];
        for (pattern, name, listed) in dotted {
            assert_eq!(matches(pattern, name), listed, "{pattern:?} {name:?}");
        }
    }

    // The rdpdr channel with this drive behind it, as `attach` makes it.
    fn channel(rig: &Rig, name: &str) -> Rdpdr {
        let drive = Drive::new(rig.root.clone(), name.to_owned());
        let drives = Some(vec![(DEVICE, name.to_owned())]);
        Rdpdr::new(Box::new(drive), "wisp".to_owned()).with_drives(drives)
    }

    fn irp(file_id: u32, completion_id: u32, major: u32, minor: u32, body: &[u8]) -> Vec<u8> {
        let mut pdu = vec![0x72, 0x44, 0x52, 0x49];
        for field in [DEVICE, file_id, completion_id, major, minor] {
            pdu.extend(field.to_le_bytes());
        }
        pdu.extend(body);
        pdu
    }

    fn answer(rdpdr: &mut Rdpdr, pdu: &[u8]) -> Vec<u8> {
        let replies = rdpdr.process(pdu).expect("the channel failed");
        assert_eq!(replies.len(), 1, "one reply");
        replies[0].encode_unframed_pdu().unwrap()
    }

    fn utf16z(text: &str) -> Vec<u8> {
        text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    // `head` as u32 fields, zeros up to 32 bytes, then `tail`.
    fn padded(head: &[u32], tail: &[u8]) -> Vec<u8> {
        let mut body: Vec<u8> = head.iter().flat_map(|field| field.to_le_bytes()).collect();
        body.resize(32, 0);
        body.extend(tail);
        body
    }

    #[test]
    fn requests_this_crate_cannot_decode_get_a_status_and_the_next_is_answered() {
        let rig = Rig::new();
        let mut rdpdr = channel(&rig, "wisp");
        let path = utf16z("\\readme.txt");
        // DesiredAccess, AllocationSize, FileAttributes, SharedAccess, CreateDisposition,
        // CreateOptions, PathLength.
        let mut create = Vec::new();
        for field in [0x8000_0000u32, 0, 0, 0, 7, 1, 0x40, path.len() as u32] {
            create.extend(field.to_le_bytes());
        }
        create.extend(&path);
        let opened = answer(&mut rdpdr, &irp(0, 1, 0x00, 0, &create));
        assert_eq!(status_of(&opened), SUCCESS);
        let id = u32_at(&opened, 16);
        let pattern = utf16z("*");
        let mut listing = 37u32.to_le_bytes().to_vec();
        listing.push(1);
        listing.extend((pattern.len() as u32).to_le_bytes());
        listing.resize(32, 0);
        listing.extend(&pattern);
        let cases = [
            ("FLUSH_BUFFERS", 0x09, 0, SUCCESS, padded(&[], &[])),
            ("QUERY_SECURITY", 0x14, 0, NOT_SUPPORTED, padded(&[4, 64], &[])),
            ("SET_SECURITY", 0x15, 0, NOT_SUPPORTED, padded(&[4, 20], &[0; 20])),
            ("SET_VOLUME_INFORMATION", 0x0b, 0, NOT_SUPPORTED, padded(&[2, 10], &[0; 10])),
            ("LOCK_CONTROL", 0x11, 0, NOT_SUPPORTED, padded(&[1, 1], &[0; 16])),
            ("DEVICE_CONTROL", 0x0e, 0, NOT_SUPPORTED, padded(&[0, 0, 0x0009_00A8], &[])),
            ("an unknown major function", 0x42, 0, NOT_SUPPORTED, padded(&[], &[])),
            ("FileFsSectorSizeInformation", 0x0a, 0, NOT_SUPPORTED, padded(&[11, 0], &[])),
            ("FileDispositionInformationEx", 0x06, 0, NOT_SUPPORTED, padded(&[64, 4], &[1; 4])),
            ("FileIdBothDirectoryInformation", 0x0c, 1, NOT_SUPPORTED, listing),
            ("a cut-short read", 0x03, 0, NOT_SUPPORTED, vec![0; 3]),
        ];
        for (n, (what, major, minor, status, body)) in cases.into_iter().enumerate() {
            let completion = 10 + 2 * n as u32;
            let reply = answer(&mut rdpdr, &irp(id, completion, major, minor, &body));
            assert_eq!(&reply[..4], &[0x72, 0x44, 0x43, 0x49], "{what}");
            assert_eq!((u32_at(&reply, 4), u32_at(&reply, 8)), (DEVICE, completion), "{what}");
            assert_eq!(status_of(&reply), status, "{what}");
            let query = irp(id, completion + 1, 0x05, 0, &padded(&[4, 0], &[]));
            let basic = answer(&mut rdpdr, &query);
            assert_eq!(u32_at(&basic, 8), completion + 1, "after {what}");
            assert_eq!(status_of(&basic), SUCCESS, "after {what}");
        }
    }

    #[test]
    fn the_drive_is_announced_by_its_name() {
        let rig = Rig::new();
        let mut rdpdr = channel(&rig, "Doc_1-2");
        let announce = answer(&mut rdpdr, &[0x72, 0x44, 0x4C, 0x55]);
        assert_eq!(&announce[..4], &[0x72, 0x44, 0x41, 0x44], "a device list announce");
        let fields = [u32_at(&announce, 4), u32_at(&announce, 8), u32_at(&announce, 12)];
        assert_eq!(fields, [1, 8, DEVICE], "one drive");
        assert_eq!(&announce[16..24], b"Doc_1-2\0", "PreferredDosName");
        assert_eq!(u32_at(&announce, 24), 8);
        assert_eq!(&announce[28..], b"Doc_1-2\0", "DeviceData");
    }
}
