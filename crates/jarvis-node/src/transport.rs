//! Verschlüsselte, gegenseitig authentifizierte Verbindung zwischen zwei Geräten.
//!
//! Noise `NNpsk0`: Nur wer den gemeinsamen Pairing-Schlüssel kennt, kommt
//! durch den Handshake; danach ist jeder Frame mit ChaCha20-Poly1305
//! verschlüsselt und gegen Manipulation geschützt.

use anyhow::{anyhow, ensure, Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use jarvis_protocol::Frame;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const NOISE_PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const MAX_NOISE_MSG: usize = 65_535;
const TAG_LEN: usize = 16;

pub struct SecureConn {
    io: Framed<TcpStream, LengthDelimitedCodec>,
    noise: snow::TransportState,
    buf: Vec<u8>,
}

fn framed(stream: TcpStream) -> Framed<TcpStream, LengthDelimitedCodec> {
    let _ = stream.set_nodelay(true);
    Framed::new(
        stream,
        LengthDelimitedCodec::builder()
            .max_frame_length(MAX_NOISE_MSG)
            .new_codec(),
    )
}

fn builder(psk: &[u8; 32]) -> Result<snow::Builder<'_>> {
    Ok(snow::Builder::new(NOISE_PATTERN.parse()?).psk(0, psk))
}

impl SecureConn {
    /// Verbindungsaufbau als anrufende Seite.
    pub async fn initiate(stream: TcpStream, psk: &[u8; 32]) -> Result<Self> {
        let mut hs = builder(psk)?.build_initiator()?;
        let mut io = framed(stream);
        let mut buf = vec![0u8; MAX_NOISE_MSG];

        let n = hs.write_message(&[], &mut buf)?;
        io.send(Bytes::copy_from_slice(&buf[..n])).await?;
        let msg = io
            .next()
            .await
            .ok_or_else(|| anyhow!("Gegenstelle hat die Verbindung geschlossen"))??;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| anyhow!("Handshake fehlgeschlagen – stimmt der Pairing-Schlüssel?"))?;

        Ok(Self {
            io,
            noise: hs.into_transport_mode()?,
            buf,
        })
    }

    /// Verbindungsaufbau als angerufene Seite.
    pub async fn accept(stream: TcpStream, psk: &[u8; 32]) -> Result<Self> {
        let mut hs = builder(psk)?.build_responder()?;
        let mut io = framed(stream);
        let mut buf = vec![0u8; MAX_NOISE_MSG];

        let msg = io
            .next()
            .await
            .ok_or_else(|| anyhow!("Gegenstelle hat die Verbindung geschlossen"))??;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| anyhow!("Handshake fehlgeschlagen – stimmt der Pairing-Schlüssel?"))?;
        let n = hs.write_message(&[], &mut buf)?;
        io.send(Bytes::copy_from_slice(&buf[..n])).await?;

        Ok(Self {
            io,
            noise: hs.into_transport_mode()?,
            buf,
        })
    }

    pub async fn send(&mut self, frame: &Frame) -> Result<()> {
        let plain = frame.encode()?;
        ensure!(
            plain.len() + TAG_LEN <= MAX_NOISE_MSG,
            "Frame zu groß ({} Byte)",
            plain.len()
        );
        let n = self.noise.write_message(&plain, &mut self.buf)?;
        self.io.send(Bytes::copy_from_slice(&self.buf[..n])).await?;
        Ok(())
    }

    /// `None`, wenn die Gegenstelle die Verbindung sauber geschlossen hat.
    /// Abbruchsicher (cancel-safe) und damit in `tokio::select!` verwendbar.
    pub async fn recv(&mut self) -> Result<Option<Frame>> {
        let Some(msg) = self.io.next().await else {
            return Ok(None);
        };
        let msg = msg?;
        let n = self
            .noise
            .read_message(&msg, &mut self.buf)
            .context("Frame konnte nicht entschlüsselt werden")?;
        Ok(Some(Frame::decode(&self.buf[..n])?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn pair(a: [u8; 32], b: [u8; 32]) -> (Result<SecureConn>, Result<SecureConn>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            SecureConn::accept(s, &b).await
        });
        let client = SecureConn::initiate(TcpStream::connect(addr).await.unwrap(), &a).await;
        (client, server.await.unwrap())
    }

    #[tokio::test]
    async fn roundtrip_with_same_key() {
        let (c, s) = pair([7; 32], [7; 32]).await;
        let (mut c, mut s) = (c.unwrap(), s.unwrap());
        c.send(&Frame::Ping).await.unwrap();
        assert_eq!(s.recv().await.unwrap(), Some(Frame::Ping));
        s.send(&Frame::Pong).await.unwrap();
        assert_eq!(c.recv().await.unwrap(), Some(Frame::Pong));
    }

    #[tokio::test]
    async fn wrong_key_is_rejected() {
        let (c, s) = pair([1; 32], [2; 32]).await;
        assert!(s.is_err() || c.is_err());
    }
}
