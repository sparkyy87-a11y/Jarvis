//! Verschlüsselte, gegenseitig authentifizierte Verbindung zwischen zwei Geräten.
//!
//! Noise `XX`: Jedes Gerät hat ein eigenes Schlüsselpaar und beweist im
//! Handshake, dass es den geheimen Teil besitzt. Welches Gerät vertrauenswürdig
//! ist, entscheidet danach die Vertrauensliste (`trust`) anhand von
//! [`SecureConn::remote_key`]. Jeder Frame ist mit ChaCha20-Poly1305
//! verschlüsselt und gegen Manipulation geschützt; dank frischer
//! Sitzungsschlüssel bleiben alte Mitschnitte auch dann geheim, wenn später ein
//! Geräteschlüssel verloren geht (Forward Secrecy).

use anyhow::{anyhow, ensure, Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use mira_protocol::Frame;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_NOISE_MSG: usize = 65_535;
const TAG_LEN: usize = 16;

/// Schlüsselpaar eines Geräts (X25519).
#[derive(Clone)]
pub struct Identity {
    pub private: [u8; 32],
    pub public: [u8; 32],
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let kp = snow::Builder::new(NOISE_PATTERN.parse()?).generate_keypair()?;
        Ok(Self {
            private: kp
                .private
                .as_slice()
                .try_into()
                .context("Schlüssel hat falsche Länge")?,
            public: kp
                .public
                .as_slice()
                .try_into()
                .context("Schlüssel hat falsche Länge")?,
        })
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.public)
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Den geheimen Schlüssel nie in Logs schreiben.
        write!(f, "Identity({})", self.public_hex())
    }
}

pub struct SecureConn {
    io: Framed<TcpStream, LengthDelimitedCodec>,
    noise: snow::TransportState,
    buf: Vec<u8>,
    remote: [u8; 32],
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

fn builder(id: &Identity) -> Result<snow::Builder<'_>> {
    Ok(snow::Builder::new(NOISE_PATTERN.parse()?).local_private_key(&id.private))
}

async fn next_msg(io: &mut Framed<TcpStream, LengthDelimitedCodec>) -> Result<bytes::BytesMut> {
    io.next()
        .await
        .ok_or_else(|| anyhow!("Gegenstelle hat die Verbindung geschlossen"))?
        .map_err(Into::into)
}

fn finish(
    hs: snow::HandshakeState,
    io: Framed<TcpStream, LengthDelimitedCodec>,
    buf: Vec<u8>,
) -> Result<SecureConn> {
    let remote: [u8; 32] = hs
        .get_remote_static()
        .context("Gegenstelle hat keinen Geräteschlüssel gezeigt")?
        .try_into()
        .context("Geräteschlüssel hat falsche Länge")?;
    Ok(SecureConn {
        io,
        noise: hs.into_transport_mode()?,
        buf,
        remote,
    })
}

impl SecureConn {
    /// Verbindungsaufbau als anrufende Seite (-> e, <- e ee s es, -> s se).
    pub async fn initiate(stream: TcpStream, id: &Identity) -> Result<Self> {
        let mut hs = builder(id)?.build_initiator()?;
        let mut io = framed(stream);
        let mut buf = vec![0u8; MAX_NOISE_MSG];

        let n = hs.write_message(&[], &mut buf)?;
        io.send(Bytes::copy_from_slice(&buf[..n])).await?;
        let msg = next_msg(&mut io).await?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| anyhow!("Handshake fehlgeschlagen"))?;
        let n = hs.write_message(&[], &mut buf)?;
        io.send(Bytes::copy_from_slice(&buf[..n])).await?;
        finish(hs, io, buf)
    }

    /// Verbindungsaufbau als angerufene Seite.
    pub async fn accept(stream: TcpStream, id: &Identity) -> Result<Self> {
        let mut hs = builder(id)?.build_responder()?;
        let mut io = framed(stream);
        let mut buf = vec![0u8; MAX_NOISE_MSG];

        let msg = next_msg(&mut io).await?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| anyhow!("Handshake fehlgeschlagen"))?;
        let n = hs.write_message(&[], &mut buf)?;
        io.send(Bytes::copy_from_slice(&buf[..n])).await?;
        let msg = next_msg(&mut io).await?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| anyhow!("Handshake fehlgeschlagen"))?;
        finish(hs, io, buf)
    }

    /// Öffentlicher Schlüssel der Gegenstelle (im Handshake bewiesen), hex.
    pub fn remote_key(&self) -> String {
        hex::encode(self.remote)
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

    async fn pair(a: Identity, b: Identity) -> (SecureConn, SecureConn) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            SecureConn::accept(s, &b).await
        });
        let client = SecureConn::initiate(TcpStream::connect(addr).await.unwrap(), &a).await;
        (client.unwrap(), server.await.unwrap().unwrap())
    }

    #[tokio::test]
    async fn both_sides_learn_the_proven_key() {
        let (a, b) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let (mut c, mut s) = pair(a.clone(), b.clone()).await;
        assert_eq!(c.remote_key(), b.public_hex());
        assert_eq!(s.remote_key(), a.public_hex());
        c.send(&Frame::Ping).await.unwrap();
        assert_eq!(s.recv().await.unwrap(), Some(Frame::Ping));
        s.send(&Frame::Pong).await.unwrap();
        assert_eq!(c.recv().await.unwrap(), Some(Frame::Pong));
    }

    #[tokio::test]
    async fn debug_never_prints_secret() {
        let id = Identity::generate().unwrap();
        let shown = format!("{id:?}");
        assert!(!shown.contains(&hex::encode(id.private)));
    }
}
