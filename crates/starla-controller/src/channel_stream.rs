//! Async stream adapter for SSH channels
//!
//! Bridges a `russh::Channel<Msg>` into a standard `DuplexStream` that
//! implements `AsyncRead + AsyncWrite`, usable anywhere that expects a
//! tokio async stream.

use std::time::Duration;

use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::time::Instant;
use tracing::{debug, trace};

/// How long a channel the app has closed stays open for the server to close
/// its side. OpenSSH's ssh waits for as long as it takes; this bounds a
/// server that never does.
const CLOSE_GRACE: Duration = Duration::from_secs(30);

/// Create a bidirectional async stream from an SSH channel.
///
/// Returns a `DuplexStream` that can be used as a normal async stream.
/// A background task bridges data between the SSH channel and the stream.
///
/// The channel ends as OpenSSH's ssh ends a forwarded connection (nchan.c):
/// one EOF each way, then one CLOSE. When the server's EOF comes first, the
/// app's stream ends, and the bridge sends its EOF and CLOSE. When the app
/// closes its side first (shuts down its writing half, or drops the stream),
/// the bridge sends its EOF and goes on passing the server's data to the app
/// until the server's EOF, then sends CLOSE: at most [`CLOSE_GRACE`] later,
/// or at once if the server has more to say and the app is gone. Each is
/// sent once: Starla sent EOF twice and left CLOSE to russh's answer to the
/// server's. Once the server's CLOSE has come, russh has answered it, and
/// the bridge sends nothing more.
pub fn channel_to_stream(mut channel: Channel<Msg>) -> DuplexStream {
    let (app_side, bridge_side) = tokio::io::duplex(64 * 1024);
    let (mut bridge_read, mut bridge_write) = tokio::io::split(bridge_side);

    tokio::spawn(async move {
        let mut local_buf = [0u8; 8192];
        // Set when the app has closed its side and our EOF has gone: the
        // server has until then to close its side
        let mut closing_by: Option<Instant> = None;

        loop {
            tokio::select! {
                biased;

                // SSH channel -> app
                msg = channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { data }) => {
                            trace!("SSH channel -> stream: {} bytes", data.len());
                            if bridge_write.write_all(&data).await.is_err() {
                                // The app is gone: nothing more can reach it
                                break;
                            }
                            let _ = bridge_write.flush().await;
                        }
                        Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                            debug!("SSH channel closed");
                            let _ = bridge_write.shutdown().await;
                            break;
                        }
                        _ => {} // ignore other messages
                    }
                }

                // App -> SSH channel, until the app closes its side
                result = bridge_read.read(&mut local_buf), if closing_by.is_none() => {
                    match result {
                        Ok(0) => {
                            debug!("Stream closed by app");
                            let _ = channel.eof().await;
                            closing_by = Some(Instant::now() + CLOSE_GRACE);
                        }
                        Ok(n) => {
                            trace!("Stream -> SSH channel: {} bytes", n);
                            if channel.data(&local_buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                () = tokio::time::sleep_until(closing_by.unwrap_or_else(Instant::now)),
                    if closing_by.is_some() =>
                {
                    debug!(
                        "SSH channel still open {:?} after the app closed its side",
                        CLOSE_GRACE
                    );
                    break;
                }
            }
        }

        // Our EOF unless the app's close has sent it, then CLOSE: each once
        if closing_by.is_none() {
            let _ = channel.eof().await;
        }
        let _ = channel.close().await;
        debug!("Channel stream bridge ended");
    });

    app_side
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use russh::keys::PrivateKeyWithHashAlg;
    use russh::server::{Auth, Session};
    use russh::ChannelId;

    /// What the server received on the channel from the probe, in order
    type Seen = Arc<Mutex<Vec<&'static str>>>;

    /// How the server answers the probe's EOF
    #[derive(Clone, Copy)]
    enum Answer {
        /// It does not: it never closes
        Never,
        /// Its reply (if any) and its EOF, then its CLOSE once the probe's
        /// has come (russh's server answers a CLOSE itself)
        Eof(&'static [u8]),
        /// As OpenSSH does once the connection it forwards to has replied
        /// and closed: the reply (if any), its EOF and its CLOSE at once
        EofClose(&'static [u8]),
    }

    /// The server's end of a channel. It records each EOF and CLOSE the probe
    /// sends while the channel is open on the server's side: since russh
    /// 0.62.5 (GHSA-m65r-rprj-r5rg) its server drops whatever comes for a
    /// channel after its own CLOSE, so a probe's CLOSE that answers the
    /// server's never reaches the handler. An exec gets "pong" and the
    /// server's EOF and CLOSE at once.
    struct Far {
        seen: Seen,
        answer: Answer,
        channels: Vec<Channel<russh::server::Msg>>,
    }

    impl russh::server::Handler for Far {
        type Error = anyhow::Error;

        async fn auth_publickey(
            &mut self,
            _: &str,
            _: russh::server::VerifiedPublicKey<'_>,
        ) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            channel: Channel<russh::server::Msg>,
            reply: russh::server::ChannelOpenHandle,
            _: &mut Session,
        ) -> Result<(), Self::Error> {
            self.channels.push(channel);
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            _: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            session.data(channel, b"pong\n".to_vec())?;
            session.eof(channel)?;
            session.close(channel)?;
            Ok(())
        }

        async fn channel_eof(
            &mut self,
            channel: ChannelId,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.seen.lock().unwrap().push("eof");
            let (reply, close) = match self.answer {
                Answer::Never => return Ok(()),
                Answer::Eof(reply) => (reply, false),
                Answer::EofClose(reply) => (reply, true),
            };
            if !reply.is_empty() {
                session.data(channel, reply.to_vec())?;
            }
            session.eof(channel)?;
            if close {
                session.close(channel)?;
            }
            Ok(())
        }

        async fn channel_close(
            &mut self,
            _: ChannelId,
            _: &mut Session,
        ) -> Result<(), Self::Error> {
            self.seen.lock().unwrap().push("close");
            Ok(())
        }
    }

    struct Near;

    impl russh::client::Handler for Near {
        type Error = anyhow::Error;

        async fn check_server_key(
            &mut self,
            _: &russh::keys::PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    /// A channel to a [`Far`] server over an in-memory wire, what the server
    /// sees the probe send on it, and the session's handle (dropping it
    /// ends the session).
    async fn channel_to(answer: Answer) -> (Channel<Msg>, Seen, russh::client::Handle<Near>) {
        let (near, far) = tokio::io::duplex(1 << 16);
        let seen = Seen::default();
        let server = russh::server::Config {
            keys: vec![crate::generate_key().unwrap()],
            inactivity_timeout: None,
            keepalive_interval: None,
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        };
        let handler = Far {
            seen: seen.clone(),
            answer,
            channels: Vec::new(),
        };
        tokio::spawn(async move {
            if let Ok(running) = russh::server::run_stream(Arc::new(server), far, handler).await {
                let _ = running.await;
            }
        });
        let client = russh::client::Config {
            inactivity_timeout: None,
            ..Default::default()
        };
        let mut handle = russh::client::connect_stream(Arc::new(client), near, Near)
            .await
            .unwrap();
        let key = crate::generate_key().unwrap();
        let auth = handle
            .authenticate_publickey("atlas", PrivateKeyWithHashAlg::new(Arc::new(key), None))
            .await
            .unwrap();
        assert!(auth.success());
        let channel = handle.channel_open_session().await.unwrap();
        (channel, seen, handle)
    }

    /// What the server has seen once the probe's CLOSE has come, and a
    /// moment more for anything after it
    async fn closed(seen: &Seen) -> Vec<&'static str> {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !seen.lock().unwrap().contains(&"close") {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the probe closes the channel");
        tokio::time::sleep(Duration::from_millis(50)).await;
        seen.lock().unwrap().clone()
    }

    /// What the server has seen a moment after the stream has gone, for a
    /// channel the server has closed: nothing after its CLOSE can show here
    async fn settled(seen: &Seen) -> Vec<&'static str> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        seen.lock().unwrap().clone()
    }

    /// The SSH session is still up: a new channel's exec gets its "pong".
    /// RIPE's controller dropped the whole session after a channel close it
    /// did not like.
    async fn still_up(handle: &russh::client::Handle<Near>) {
        let mut channel = handle.channel_open_session().await.unwrap();
        channel.exec(true, "KEEP").await.unwrap();
        let mut out = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        assert_eq!(out, b"pong\n");
    }

    #[tokio::test]
    async fn an_app_that_closes_first_sends_one_eof_and_one_close() {
        // The telnet handler dropping its stream: Starla sent EOF twice and
        // no CLOSE of its own
        let (channel, seen, handle) = channel_to(Answer::Eof(b"")).await;
        drop(channel_to_stream(channel));
        assert_eq!(closed(&seen).await, ["eof", "close"]);
        still_up(&handle).await;
    }

    #[tokio::test]
    async fn an_app_that_closes_first_sends_one_eof_to_a_server_that_closes_at_once() {
        // OpenSSH's order: the server's EOF and CLOSE together. russh
        // answers the CLOSE; the bridge sends nothing more, and the session
        // stays up.
        let (channel, seen, handle) = channel_to(Answer::EofClose(b"")).await;
        drop(channel_to_stream(channel));
        assert_eq!(settled(&seen).await, ["eof"]);
        still_up(&handle).await;
    }

    #[tokio::test]
    async fn an_app_that_shuts_down_its_side_still_reads_the_reply() {
        // A request and its end, then the server's reply after our EOF
        let (channel, seen, _handle) = channel_to(Answer::Eof(b"reply")).await;
        let mut stream = channel_to_stream(channel);
        stream.write_all(b"request").await.unwrap();
        stream.shutdown().await.unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"reply");
        assert_eq!(closed(&seen).await, ["eof", "close"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_never_closes_gets_the_close_after_the_grace() {
        let (channel, seen, _handle) = channel_to(Answer::Never).await;
        drop(channel_to_stream(channel));
        tokio::time::sleep(CLOSE_GRACE - Duration::from_secs(1)).await;
        assert_eq!(*seen.lock().unwrap(), ["eof"]);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(closed(&seen).await, ["eof", "close"]);
    }

    #[tokio::test]
    async fn a_server_that_closes_first_keeps_the_session() {
        // As the controller ends a telnet connection: its data, EOF and
        // CLOSE, which russh answers itself. Whatever the bridge sends after
        // that goes nowhere (russh's client drops a closed channel's EOF and
        // CLOSE, Encrypted::byte), and the session stays up.
        let (channel, seen, handle) = channel_to(Answer::Never).await;
        channel.exec(true, "KEEP").await.unwrap();
        let mut stream = channel_to_stream(channel);
        let mut out = Vec::new();
        stream.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"pong\n");
        drop(stream);
        let seen = settled(&seen).await;
        assert!(
            seen.iter().filter(|s| **s == "eof").count() <= 1,
            "{seen:?}"
        );
        still_up(&handle).await;
    }
}
