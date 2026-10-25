use std::time::Duration;

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrSvcMessages, Client};
use ironrdp_connector::ClientConnector;
use ironrdp_pdu::{PduErrorKind, PduResult};
use ironrdp_session::{ActiveStage, SessionError, SessionErrorExt as _, SessionResult};
use tokio::sync::mpsc;
use tokio::time::Instant;

pub(super) const MAX_TEXT: usize = 1 << 20;
pub(super) const PASTE_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug)]
enum Event {
    SendFormats,
    Ready,
    RemoteText,
    Text(Option<String>),
    Wanted(ClipboardFormatId),
}

#[derive(Debug)]
struct Backend {
    events: mpsc::UnboundedSender<Event>,
    inbox: Option<mpsc::UnboundedReceiver<Event>>,
}

ironrdp_core::impl_as_any!(Backend);

impl Backend {
    fn new() -> Self {
        let (events, inbox) = mpsc::unbounded_channel();
        Backend { events, inbox: Some(inbox) }
    }

    // The session task reads these after the call that produced them returns.
    fn tell(&self, event: Event) {
        self.events.send(event).ok();
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        self.tell(Event::Ready);
    }

    fn on_request_format_list(&mut self) {
        self.tell(Event::SendFormats);
    }

    fn on_process_negotiated_capabilities(&mut self, _flags: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if available_formats.iter().any(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT) {
            self.tell(Event::RemoteText);
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.tell(Event::Wanted(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let text = if response.is_error() { None } else { from_remote(response.data()) };
        self.tell(Event::Text(text));
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

pub(super) fn attach(connector: ClientConnector) -> ClientConnector {
    connector.with_static_channel(CliprdrClient::new(Box::new(Backend::new())))
}

pub(super) enum Step {
    Events,
    Copy(Option<String>),
    Timeouts,
    Discard,
}

pub(super) struct Clipboard {
    inbox: Option<mpsc::UnboundedReceiver<Event>>,
    held: Option<String>,
    listed: bool,
    ready: bool,
    wanted: bool,
    asked: Option<Instant>,
    limit: Duration,
}

impl Clipboard {
    // The inbox belongs to the backend until the session starts; without the channel (a
    // connector built without `attach`) nothing happens.
    pub(super) fn take(stage: &mut ActiveStage, limit: Duration) -> Self {
        let inbox = stage
            .get_svc_processor_mut::<CliprdrClient>()
            .and_then(|channel| channel.downcast_backend_mut::<Backend>())
            .and_then(|backend| backend.inbox.take());
        Clipboard {
            inbox,
            held: None,
            listed: false,
            ready: false,
            wanted: false,
            asked: None,
            limit,
        }
    }

    // Returns the bytes to write. A text from the remote is left in `received`.
    pub(super) fn step(
        &mut self,
        stage: &mut ActiveStage,
        step: Step,
        received: &mut Option<String>,
    ) -> SessionResult<Vec<u8>> {
        let mut out = Vec::new();
        match step {
            Step::Events => {
                while let Some(event) = self.inbox.as_mut().and_then(|inbox| inbox.try_recv().ok())
                {
                    self.event(event, stage, received, &mut out)?;
                }
            }
            Step::Copy(text) => {
                // A text that is too long withdraws the one held before.
                let changed = text.is_some() || self.held.is_some();
                self.held = text;
                if changed && self.ready {
                    self.announce(stage, &mut out)?;
                }
            }
            Step::Discard => {
                while self.inbox.as_mut().is_some_and(|inbox| inbox.try_recv().is_ok()) {}
            }
            Step::Timeouts => {
                if let Some(channel) = stage.get_svc_processor_mut::<CliprdrClient>() {
                    let messages = channel.drive_timeouts();
                    sent(stage, messages, &mut out)?;
                }
                self.request(stage, &mut out)?;
            }
        }
        Ok(out)
    }

    fn event(
        &mut self,
        event: Event,
        stage: &mut ActiveStage,
        received: &mut Option<String>,
        out: &mut Vec<u8>,
    ) -> SessionResult<()> {
        match event {
            Event::SendFormats => self.announce(stage, out)?,
            Event::Ready => {
                self.ready = true;
                if self.held.is_some() && !self.listed {
                    self.announce(stage, out)?;
                }
            }
            Event::RemoteText => self.wanted = true,
            Event::Text(text) => {
                // A newer copy is waiting, so this text is already old.
                let current = self.asked.take().is_some() && !self.wanted;
                if let Some(text) = text.filter(|_| current) {
                    *received = Some(text);
                }
            }
            Event::Wanted(format) => {
                let response = match &self.held {
                    Some(text) if format == ClipboardFormatId::CF_UNICODETEXT => {
                        FormatDataResponse::new_unicode_string(text)
                    }
                    _ => FormatDataResponse::new_error(),
                };
                if let Some(channel) = stage.get_svc_processor_mut::<CliprdrClient>() {
                    let messages = channel.submit_format_data(response);
                    sent(stage, messages, out)?;
                }
            }
        }
        self.request(stage, out)
    }

    // The crate never gives up on a request, and a server may leave one unanswered.
    fn request(&mut self, stage: &mut ActiveStage, out: &mut Vec<u8>) -> SessionResult<()> {
        if self.asked.is_some_and(|asked| asked.elapsed() > self.limit) {
            self.asked = None;
        }
        if self.wanted && self.asked.is_none() && self.ready {
            if let Some(channel) = stage.get_svc_processor_mut::<CliprdrClient>() {
                let messages = channel.initiate_paste(ClipboardFormatId::CF_UNICODETEXT);
                let taken = sent(stage, messages, out)?;
                self.asked = taken.then(Instant::now);
                self.wanted = !taken;
            }
        }
        Ok(())
    }

    fn announce(&mut self, stage: &mut ActiveStage, out: &mut Vec<u8>) -> SessionResult<()> {
        let formats: Vec<_> = self
            .held
            .iter()
            .map(|_| ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT))
            .collect();
        self.listed = !formats.is_empty();
        if let Some(channel) = stage.get_svc_processor_mut::<CliprdrClient>() {
            let messages = channel.initiate_copy(&formats);
            sent(stage, messages, out)?;
        }
        Ok(())
    }
}

// The remote text for the view: no NULs, LF line ends, nothing empty or over the limit. NULs are
// dropped here and not by the crate, which would cut the text at the first one.
fn from_remote(data: &[u8]) -> Option<String> {
    let units = data.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    let text: String = char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .filter(|unit| *unit != '\0')
        .collect();
    let text = text.replace("\r\n", "\n");
    (!text.is_empty() && text.len() <= MAX_TEXT).then_some(text)
}

// The local text as the remote wants it: CR LF line ends.
pub(super) fn to_remote(text: &str) -> Option<String> {
    (text.len() <= MAX_TEXT).then(|| text.replace("\r\n", "\n").replace('\n', "\r\n"))
}

// The crate refuses what it is asked before the channel is ready, which is not a failure of the
// session; the answer says whether the messages were taken.
fn sent(
    stage: &ActiveStage,
    messages: PduResult<CliprdrSvcMessages<Client>>,
    out: &mut Vec<u8>,
) -> SessionResult<bool> {
    let messages = match messages {
        Ok(messages) => messages,
        Err(e)
            if matches!(e.kind(), PduErrorKind::Other { description }
                if description.ends_with("not in Ready state")) =>
        {
            return Ok(false);
        }
        Err(e) => return Err(SessionError::pdu(e)),
    };
    out.extend(stage.process_svc_processor_messages(messages)?);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use ironrdp_session::ActiveStageBuilder;
    use ironrdp_svc::StaticChannelSet;

    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn discarding_empties_the_inbox() {
        let mut backend = Backend::new();
        let mut clipboard = Clipboard {
            inbox: backend.inbox.take(),
            held: None,
            listed: false,
            ready: true,
            wanted: false,
            asked: None,
            limit: PASTE_LIMIT,
        };
        for _ in 0..3 {
            backend.tell(Event::Text(Some("text".into())));
        }
        let mut stage = ActiveStageBuilder {
            static_channels: StaticChannelSet::new(),
            user_channel_id: 1002,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 0,
            compression_type: None,
            enable_server_pointer: true,
            pointer_software_rendering: false,
        }
        .build();
        let mut received = None;
        let out = clipboard.step(&mut stage, Step::Discard, &mut received);
        assert!(out.unwrap().is_empty());
        assert_eq!(received, None);
        assert!(clipboard.inbox.as_mut().unwrap().try_recv().is_err());
    }

    #[test]
    fn remote_text_loses_nuls_and_gets_line_feeds() {
        let mixed = from_remote(&utf16("a\r\nb\r\n\r\nc\rd\ne"));
        assert_eq!(mixed.as_deref(), Some("a\nb\n\nc\rd\ne"));
        assert_eq!(from_remote(&utf16("a\0b\0\0")).as_deref(), Some("ab"));
        let crab = from_remote(&utf16("from the server \u{1f980}"));
        assert_eq!(crab.as_deref(), Some("from the server \u{1f980}"));
    }

    #[test]
    fn remote_text_that_is_empty_or_too_long_is_dropped() {
        assert_eq!(from_remote(&[]), None);
        assert_eq!(from_remote(&[0, 0]), None);
        assert_eq!(from_remote(&[0, 0, 0, 0]), None);
        let limit = from_remote(&utf16(&"x".repeat(MAX_TEXT)));
        assert_eq!(limit.map(|text| text.len()), Some(MAX_TEXT));
        assert_eq!(from_remote(&utf16(&"x".repeat(MAX_TEXT + 1))), None);
    }

    #[test]
    fn remote_text_with_broken_utf16_is_kept_with_replacements() {
        let lone_surrogate = [0x00, 0xd8, b'b', 0];
        assert_eq!(from_remote(&lone_surrogate).as_deref(), Some("\u{fffd}b"));
        assert_eq!(from_remote(&[b'a', 0, 0x41]).as_deref(), Some("a"));
    }

    #[test]
    fn local_text_gets_carriage_returns_and_the_limit() {
        assert_eq!(to_remote("x\ny").as_deref(), Some("x\r\ny"));
        assert_eq!(to_remote("x\r\ny\n\nz\r").as_deref(), Some("x\r\ny\r\n\r\nz\r"));
        assert_eq!(to_remote("").as_deref(), Some(""));
        assert_eq!(to_remote(&"x".repeat(MAX_TEXT)).map(|text| text.len()), Some(MAX_TEXT));
        assert_eq!(to_remote(&"x".repeat(MAX_TEXT + 1)), None);
    }
}
