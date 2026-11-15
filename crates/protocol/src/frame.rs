use crate::Message;
use thiserror::Error;

/// Upper bound for one frame. Oversized frames are rejected before allocation.
pub const MAX_FRAME_LEN: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame of {0} bytes exceeds limit of {MAX_FRAME_LEN}")]
    TooLarge(usize),
    #[error("empty frame")]
    Empty,
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("connection closed")]
    Closed,
}

/// Serialize a message into a length-prefixed frame.
pub fn encode_frame(msg: &Message) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode a frame payload (without the length prefix).
pub fn decode_payload(payload: &[u8]) -> Result<Message, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::Empty);
    }
    Ok(serde_json::from_slice(payload)?)
}

#[cfg(feature = "tokio")]
pub async fn read_frame<R>(r: &mut R) -> Result<Message, FrameError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(FrameError::Closed),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    if len == 0 {
        return Err(FrameError::Empty);
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    decode_payload(&buf)
}

#[cfg(feature = "tokio")]
pub async fn write_frame<W>(w: &mut W, msg: &Message) -> Result<(), FrameError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    let frame = encode_frame(msg)?;
    w.write_all(&frame).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[tokio::test]
    async fn round_trip_over_duplex() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let msg = Message::Ping { seq: 7 };
        write_frame(&mut a, &msg).await.unwrap();
        let got = read_frame(&mut b).await.unwrap();
        assert_eq!(got, msg);
    }

    #[tokio::test]
    async fn rejects_oversized_frame_without_allocating() {
        let (mut a, mut b) = tokio::io::duplex(64);
        use tokio::io::AsyncWriteExt;
        a.write_all(&((MAX_FRAME_LEN as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut b).await,
            Err(FrameError::TooLarge(_))
        ));
    }

    #[tokio::test]
    async fn rejects_garbage() {
        let (mut a, mut b) = tokio::io::duplex(64);
        use tokio::io::AsyncWriteExt;
        a.write_all(&3u32.to_be_bytes()).await.unwrap();
        a.write_all(b"{x}").await.unwrap();
        assert!(matches!(
            read_frame(&mut b).await,
            Err(FrameError::Malformed(_))
        ));
    }

    #[tokio::test]
    async fn closed_stream_reports_closed() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        assert!(matches!(read_frame(&mut b).await, Err(FrameError::Closed)));
    }
}
