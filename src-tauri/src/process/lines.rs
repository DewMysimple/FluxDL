use crate::errors::{AppError, ErrorCode, ErrorPhase};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputStream {
    Stdout,
    Stderr,
}
#[derive(Debug)]
pub struct OutputLine {
    pub stream: OutputStream,
    pub sequence: u64,
    pub text: String,
    pub truncated: bool,
}
#[derive(Debug)]
pub enum ProcessEvent {
    Line(OutputLine),
    ReadFault(AppError),
}

pub(super) async fn read_lines<R: AsyncRead + Unpin>(
    mut reader: R,
    stream: OutputStream,
    sender: mpsc::Sender<ProcessEvent>,
    phase: ErrorPhase,
) {
    let mut chunk = [0u8; 4096];
    let mut line = Vec::with_capacity(8192);
    let mut truncated = false;
    let mut sequence = 0u64;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => {
                for byte in &chunk[..count] {
                    if matches!(*byte, b'\n' | b'\r') {
                        if !line.is_empty() || truncated {
                            sequence = sequence.saturating_add(1);
                            if sender
                                .send(ProcessEvent::Line(OutputLine {
                                    stream,
                                    sequence,
                                    text: String::from_utf8_lossy(&line).into_owned(),
                                    truncated,
                                }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            line.clear();
                            truncated = false;
                        }
                    } else if line.len() < 8192 {
                        line.push(*byte);
                    } else {
                        truncated = true;
                    }
                }
            }
            Err(error) => {
                let _ = sender
                    .send(ProcessEvent::ReadFault(
                        AppError::new(ErrorCode::ProcessRead, phase, "读取引擎输出失败。")
                            .with_details(error.to_string()),
                    ))
                    .await;
                return;
            }
        }
    }
    if !line.is_empty() || truncated {
        let _ = sender
            .send(ProcessEvent::Line(OutputLine {
                stream,
                sequence: sequence.saturating_add(1),
                text: String::from_utf8_lossy(&line).into_owned(),
                truncated,
            }))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn drains_oversized_lines_without_silently_losing_truncation_or_stream_order() {
        let mut input = vec![b'x'; 20000];
        input.extend_from_slice(b"\r\nnext\n");
        let (sender, mut receiver) = mpsc::channel(4);
        read_lines(
            input.as_slice(),
            OutputStream::Stderr,
            sender,
            ErrorPhase::Download,
        )
        .await;
        let Some(ProcessEvent::Line(first)) = receiver.recv().await else {
            panic!("first line")
        };
        assert_eq!(
            (
                first.text.len(),
                first.sequence,
                first.stream,
                first.truncated
            ),
            (8192, 1, OutputStream::Stderr, true)
        );
        let Some(ProcessEvent::Line(next)) = receiver.recv().await else {
            panic!("next line")
        };
        assert_eq!(
            (next.text.as_str(), next.sequence, next.truncated),
            ("next", 2, false)
        );
        assert!(receiver.recv().await.is_none());
    }
    #[tokio::test]
    async fn read_fault_is_an_explicit_event() {
        struct Broken;
        impl AsyncRead for Broken {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Err(std::io::Error::other("fixture read fault")))
            }
        }
        let (sender, mut receiver) = mpsc::channel(1);
        read_lines(Broken, OutputStream::Stdout, sender, ErrorPhase::Download).await;
        assert!(matches!(
            receiver.recv().await,
            Some(ProcessEvent::ReadFault(AppError {
                code: ErrorCode::ProcessRead,
                ..
            }))
        ));
    }
}
