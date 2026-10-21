//! Bounded JSON-lines framing; discard an oversized frame before accepting another.
use std::io;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RequestLine {
    Eof,
    Line(Vec<u8>),
    TooLong,
}

pub(super) async fn read_request_line(reader: &mut (impl AsyncBufRead + Unpin), limit: usize) -> io::Result<RequestLine> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(if oversized {
                RequestLine::TooLong
            } else if bytes.is_empty() {
                RequestLine::Eof
            } else {
                RequestLine::Line(bytes)
            });
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let consumed = end.map_or(chunk.len(), |end| end + 1);
        if !oversized {
            if bytes.len().checked_add(consumed).is_none_or(|size| size > limit) {
                oversized = true;
                bytes.clear();
            } else {
                bytes.try_reserve_exact(consumed).map_err(io::Error::other)?;
                bytes.extend_from_slice(chunk.get(..consumed).ok_or_else(|| io::Error::other("invalid frame boundary"))?);
            }
        }
        reader.consume(consumed);
        if end.is_some() {
            return Ok(if oversized { RequestLine::TooLong } else { RequestLine::Line(bytes) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn exact_boundary_overflow_and_following_frames() {
        let mut input = BufReader::with_capacity(3, &b"1234567\n12345678\n{}\n"[..]);
        assert_eq!(read_request_line(&mut input, 8).await.unwrap(), RequestLine::Line(b"1234567\n".to_vec()));
        assert_eq!(read_request_line(&mut input, 8).await.unwrap(), RequestLine::TooLong);
        assert_eq!(read_request_line(&mut input, 8).await.unwrap(), RequestLine::Line(b"{}\n".to_vec()));
        assert_eq!(read_request_line(&mut input, 8).await.unwrap(), RequestLine::Eof);
    }

    #[tokio::test]
    async fn huge_unterminated_frame_and_split_unicode_are_bounded() {
        let huge = vec![b'x'; 65_536];
        let mut input = BufReader::with_capacity(7, huge.as_slice());
        assert_eq!(read_request_line(&mut input, 8).await.unwrap(), RequestLine::TooLong);
        assert_eq!(read_request_line(&mut input, 8).await.unwrap(), RequestLine::Eof);
        let mut input = BufReader::with_capacity(1, "éé\n{}".as_bytes());
        assert_eq!(read_request_line(&mut input, 4).await.unwrap(), RequestLine::TooLong);
        assert_eq!(read_request_line(&mut input, 4).await.unwrap(), RequestLine::Line(b"{}".to_vec()));
        let mut empty = BufReader::new(&b""[..]);
        assert_eq!(read_request_line(&mut empty, 0).await.unwrap(), RequestLine::Eof);
    }
}
