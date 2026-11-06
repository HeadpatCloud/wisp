use std::borrow::Cow;

use ironrdp_core::{Decode as _, EncodeResult, ReadCursor, cast_length, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use tracing::{debug, warn};

use crate::pdu::{self, AudioFormat, PitchPdu, ServerAudioFormatPdu, TrainingPdu, VolumePdu, WaveInfoPdu};
use crate::server::RdpsndSvcMessages;

const SNDC_WAVE: u8 = 0x02;
const HEADER_SIZE: usize = 4;
const WAVE_INFO_SIZE: usize = 16;
// A Wave PDU starts with 4 bytes of padding, where the 4 data bytes of its WaveInfo PDU belong.
const WAVE_PADDING: usize = 4;

pub trait RdpsndClientHandler: Send + core::fmt::Debug {
    fn get_flags(&self) -> pdu::AudioFormatFlags {
        pdu::AudioFormatFlags::empty()
    }

    fn get_formats(&self) -> &[AudioFormat];

    fn wave(&mut self, format_no: usize, ts: u32, data: Cow<'_, [u8]>);

    fn set_volume(&mut self, volume: VolumePdu);

    fn set_pitch(&mut self, pitch: PitchPdu);

    fn close(&mut self);
}

#[derive(Debug)]
pub struct NoopRdpsndBackend;

impl RdpsndClientHandler for NoopRdpsndBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &[]
    }

    fn wave(&mut self, _format_no: usize, _ts: u32, _data: Cow<'_, [u8]>) {}

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RdpsndState {
    Start,
    WaitingForTraining,
    Ready,
}

/// Required for rdpdr to work: [\[MS-RDPEFS\] Appendix A<1>]
///
/// [\[MS-RDPEFS\] Appendix A<1>]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
#[derive(Debug)]
pub struct Rdpsnd {
    handler: Box<dyn RdpsndClientHandler>,
    state: RdpsndState,
    server_format: Option<ServerAudioFormatPdu>,
    wave_info: Option<WaveInfoPdu>,
}

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        Self {
            handler,
            state: RdpsndState::Start,
            server_format: None,
            wave_info: None,
        }
    }

    pub fn get_format(&self, format_no: u16) -> PduResult<&AudioFormat> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no format"))?;

        server_format
            .formats
            .get(usize::from(format_no))
            .ok_or_else(|| pdu_other_err!("invalid format"))
    }

    pub fn version(&self) -> PduResult<pdu::Version> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no version"))?;

        Ok(server_format.version)
    }

    pub fn client_formats(&mut self) -> PduResult<RdpsndSvcMessages> {
        // Windows seems to be confused if the client replies with more formats, or unknown formats (e.g.: opus).
        // We ensure to only send supported formats in common with the server.
        let server_format = &self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no server format"))?
            .formats;
        let formats = self
            .handler
            .get_formats()
            .iter()
            .filter(|format| server_format.contains(format))
            .cloned()
            .collect();

        let pdu = pdu::ClientAudioFormatPdu {
            version: self.version()?,
            flags: self.handler.get_flags() | pdu::AudioFormatFlags::ALIVE,
            formats,
            volume_left: 0xFFFF,
            volume_right: 0xFFFF,
            pitch: 0x00010000,
            dgram_port: 0,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::AudioFormat(pdu).into(),
        ]))
    }

    pub fn quality_mode(&mut self) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::QualityModePdu {
            quality_mode: pdu::QualityMode::High,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::QualityMode(pdu).into(),
        ]))
    }

    pub fn training_confirm(&mut self, pdu: &TrainingPdu) -> PduResult<RdpsndSvcMessages> {
        let pack_size: EncodeResult<_> = cast_length!("wPackSize", pdu.data.len());
        let pack_size = pack_size.map_err(|e| encode_err!(e))?;
        let pdu = pdu::TrainingConfirmPdu {
            timestamp: pdu.timestamp,
            pack_size,
        };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::TrainingConfirm(pdu).into(),
        ]))
    }

    pub fn wave_confirm(&mut self, timestamp: u16, block_no: u8) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::WaveConfirmPdu { timestamp, block_no };
        Ok(RdpsndSvcMessages::new(vec![
            pdu::ClientAudioOutputPdu::WaveConfirm(pdu).into(),
        ]))
    }

    fn play(
        &mut self,
        (timestamp, format_no, block_no): (u16, u16, u8),
        ts: u32,
        data: Cow<'_, [u8]>,
    ) -> PduResult<Vec<SvcMessage>> {
        self.handler.wave(usize::from(format_no), ts, data);
        Ok(self.wave_confirm(timestamp, block_no)?.into())
    }
}

impl_as_any!(Rdpsnd);

impl SvcProcessor for Rdpsnd {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        // xrdp sends the WaveInfo PDU and its Wave PDU as two messages, and the Wave PDU has no header.
        if let Some(info) = self.wave_info.take() {
            let Some(rest) = payload.get(WAVE_PADDING..) else {
                warn!(len = payload.len(), "Dropped a Wave PDU too short for its block");
                return Ok(vec![]);
            };
            let data = [&info.data[..], rest].concat();
            let ts = u32::from(info.timestamp);
            return self.play((info.timestamp, info.format_no, info.block_no), ts, data.into());
        }
        if payload.len() == WAVE_INFO_SIZE && payload.first() == Some(&SNDC_WAVE) {
            match WaveInfoPdu::decode(&mut ReadCursor::new(&payload[HEADER_SIZE..])) {
                Ok(info) if self.state == RdpsndState::Ready => self.wave_info = Some(info),
                _ => warn!(state = ?self.state, "Dropped a WaveInfo PDU that does not fit the state"),
            }
            return Ok(vec![]);
        }

        let pdu = match pdu::ServerAudioOutputPdu::decode(&mut ReadCursor::new(payload)) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Dropped an RDPSND PDU that does not decode");
                return Ok(vec![]);
            }
        };

        debug!(?pdu, ?self.state);
        let msg = match (self.state, pdu) {
            (state, pdu::ServerAudioOutputPdu::AudioFormat(af)) => {
                if state == RdpsndState::Ready {
                    self.handler.close();
                }
                self.server_format = Some(af);
                self.state = RdpsndState::WaitingForTraining;
                let mut msgs: Vec<SvcMessage> = self.client_formats()?.into();
                if self.version()? >= pdu::Version::V6 {
                    let mut m = self.quality_mode()?.into();
                    msgs.append(&mut m);
                }
                msgs
            }
            (RdpsndState::WaitingForTraining, pdu::ServerAudioOutputPdu::Training(pdu)) => {
                self.state = RdpsndState::Ready;
                self.training_confirm(&pdu)?.into()
            }
            (RdpsndState::Ready, pdu::ServerAudioOutputPdu::Wave2(pdu)) => {
                let ts = pdu.audio_timestamp;
                return self.play((pdu.timestamp, pdu.format_no, pdu.block_no), ts, pdu.data);
            }
            (RdpsndState::Ready, pdu::ServerAudioOutputPdu::Wave(pdu)) => {
                let ts = u32::from(pdu.timestamp);
                return self.play((pdu.timestamp, pdu.format_no, pdu.block_no), ts, pdu.data);
            }
            (RdpsndState::Ready, pdu::ServerAudioOutputPdu::Volume(pdu)) => {
                self.handler.set_volume(pdu);
                vec![]
            }
            (RdpsndState::Ready, pdu::ServerAudioOutputPdu::Pitch(pdu)) => {
                self.handler.set_pitch(pdu);
                vec![]
            }
            (RdpsndState::Ready, pdu::ServerAudioOutputPdu::Close) => {
                self.handler.close();
                vec![]
            }
            (RdpsndState::Ready, pdu::ServerAudioOutputPdu::Training(pdu)) => self.training_confirm(&pdu)?.into(),
            (state, _) => {
                warn!(?state, "Dropped an RDPSND PDU that does not fit the state");
                vec![]
            }
        };

        Ok(msg)
    }
}

impl Drop for Rdpsnd {
    fn drop(&mut self) {
        self.handler.close();
    }
}

impl SvcClientProcessor for Rdpsnd {}
