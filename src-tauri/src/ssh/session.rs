use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tokio::sync::mpsc;

use crate::error::AppResult;
use crate::ssh::client::SshHandle;

pub enum PtyMsg {
    Data(Vec<u8>),
    Eof,
    Exit(u32),
    Close,
}

pub trait PtyChannel {
    async fn next(&mut self) -> Option<PtyMsg>;
    async fn write(&mut self, data: &[u8]) -> AppResult<()>;
    async fn resize(&mut self, cols: u32, rows: u32) -> AppResult<()>;
}

pub struct RusshPty(pub Channel<Msg>);

impl PtyChannel for RusshPty {
    async fn next(&mut self) -> Option<PtyMsg> {
        loop {
            match self.0.wait().await? {
                ChannelMsg::Data { data } => return Some(PtyMsg::Data(data.to_vec())),
                ChannelMsg::ExtendedData { data, .. } => return Some(PtyMsg::Data(data.to_vec())),
                ChannelMsg::ExitStatus { exit_status } => return Some(PtyMsg::Exit(exit_status)),
                ChannelMsg::Eof => return Some(PtyMsg::Eof),
                ChannelMsg::Close => return Some(PtyMsg::Close),
                _ => continue,
            }
        }
    }

    async fn write(&mut self, data: &[u8]) -> AppResult<()> {
        self.0.data(data).await?;
        Ok(())
    }

    async fn resize(&mut self, cols: u32, rows: u32) -> AppResult<()> {
        self.0.window_change(cols, rows, 0, 0).await?;
        Ok(())
    }
}

pub async fn open_pty(handle: &SshHandle, cols: u32, rows: u32) -> AppResult<RusshPty> {
    let channel = handle.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;
    Ok(RusshPty(channel))
}

// The read loop. Generic over PtyChannel so it is testable with a fake.
// Ends when the channel closes or both input channels are dropped.
pub async fn run_session<C: PtyChannel>(
    mut channel: C,
    mut input_rx: mpsc::Receiver<Vec<u8>>,
    mut resize_rx: mpsc::Receiver<(u32, u32)>,
    output: impl Fn(Vec<u8>) + Send,
) -> Option<u32> {
    let mut exit = None;
    loop {
        tokio::select! {
            msg = channel.next() => match msg {
                Some(PtyMsg::Data(bytes)) => output(bytes),
                Some(PtyMsg::Exit(code)) => exit = Some(code),
                Some(PtyMsg::Eof) => {}
                Some(PtyMsg::Close) | None => break,
            },
            Some(buf) = input_rx.recv() => {
                let _ = channel.write(&buf).await;
            }
            Some((cols, rows)) = resize_rx.recv() => {
                let _ = channel.resize(cols, rows).await;
            }
        }
    }
    exit
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    // Fake channel: yields a queued script of PtyMsg, records writes/resizes.
    struct FakeChannel {
        script: Vec<PtyMsg>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        resizes: Arc<Mutex<Vec<(u32, u32)>>>,
    }

    impl PtyChannel for FakeChannel {
        async fn next(&mut self) -> Option<PtyMsg> {
            if self.script.is_empty() {
                // block forever once the script is exhausted (until select picks another branch)
                std::future::pending::<()>().await;
                None
            } else {
                Some(self.script.remove(0))
            }
        }
        async fn write(&mut self, data: &[u8]) -> AppResult<()> {
            self.writes.lock().unwrap().push(data.to_vec());
            Ok(())
        }
        async fn resize(&mut self, cols: u32, rows: u32) -> AppResult<()> {
            self.resizes.lock().unwrap().push((cols, rows));
            Ok(())
        }
    }

    #[tokio::test]
    async fn forwards_output_and_exit_then_closes() {
        let out = Arc::new(Mutex::new(Vec::<u8>::new()));
        let out2 = out.clone();
        let channel = FakeChannel {
            script: vec![PtyMsg::Data(b"hello".to_vec()), PtyMsg::Exit(0), PtyMsg::Close],
            writes: Arc::new(Mutex::new(vec![])),
            resizes: Arc::new(Mutex::new(vec![])),
        };
        let (_in_tx, in_rx) = mpsc::channel(8);
        let (_rs_tx, rs_rx) = mpsc::channel(8);
        let code = run_session(channel, in_rx, rs_rx, move |b| out2.lock().unwrap().extend(b)).await;
        assert_eq!(code, Some(0));
        assert_eq!(out.lock().unwrap().as_slice(), b"hello");
    }

    #[tokio::test]
    async fn forwards_input_and_resize() {
        let writes = Arc::new(Mutex::new(vec![]));
        let resizes = Arc::new(Mutex::new(vec![]));
        // Empty script -> next() stays pending forever, so the ONLY ready select!
        // branches are the queued input/resize. (select! is biased to poll next()
        // first; a scripted msg would always win and starve input/resize.) We let
        // the loop drain the queued items, then abort it.
        let channel = FakeChannel {
            script: vec![],
            writes: writes.clone(),
            resizes: resizes.clone(),
        };
        let (in_tx, in_rx) = mpsc::channel(8);
        let (rs_tx, rs_rx) = mpsc::channel(8);
        in_tx.send(b"ls\n".to_vec()).await.unwrap();
        rs_tx.send((120, 40)).await.unwrap();
        let task = tokio::spawn(run_session(channel, in_rx, rs_rx, |_| {}));
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        task.abort();
        let _ = task.await;
        assert!(writes.lock().unwrap().iter().any(|w| w == b"ls\n"));
        assert!(resizes.lock().unwrap().contains(&(120, 40)));
    }
}
