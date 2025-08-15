use std::io::BufRead;

use circular_buffer::CircularBuffer;
use winnow::{
    ascii::{alphanumeric0, alphanumeric1, line_ending, till_line_ending, Caseless}, combinator::{
        alt, cut_err, delimited, eof, opt, preceded, repeat, repeat_till, separated,
        seq, terminated, trace,
    }, error::{ContextError, ErrMode, ParserError}, prelude::*, stream::Offset as _, token::{rest, take, take_till, take_until, take_while}, Partial
};

pub struct DockerFileParser {
    buffer: Box<CircularBuffer<4096, u8>>,
}

type Stream<'i> = Partial<&'i [u8]>;
type StreamSlice<'i> = <Stream<'i> as winnow::stream::Stream>::Slice;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DockerFileInstruction {
    From { src: String, name: Option<String> },
    Other(String, String),
    Strange(String),
}

fn dockerfile_instructions<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<Option<DockerFileInstruction>, E> {
    alt((
        delimited(ws(0..), single_instruction, comment_line_end).map(Some),
        comment_line_end.value(None),
        strange_line.map(Some),
    ))
    .parse_next(input)
}

fn single_instruction<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<DockerFileInstruction, E> {
    trace(
        "single_instruction",
        alt((from_instruction, other_instruction)),
    )
        .parse_next(input)
        .map_err(|e: ErrMode<E>| match e {
            ErrMode::Cut(ie) => {
                eprintln!("[Bern Internal Dockerfile Parser Problem!]");
                ErrMode::Backtrack(ie)
            },
            _ => e,
        })
}

fn from_instruction<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<DockerFileInstruction, E> {
    let mut from_as = opt(seq!(_: ws(1..), _: Caseless("as"), _: ws(1..), from_name));
    seq!(_: Caseless("from"), _: generic_args, _: ws(1..), from_image, from_as)
        .map(|r| DockerFileInstruction::From {
            src: r.0,
            name: r.1.map(|s| s.0),
        })
        .parse_next(input)
}

fn from_image<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> {
    trace(
        "from_image",
        take_until_ws.verify_map(|s| String::from_utf8(s.to_owned()).ok()),
    )
    .parse_next(input)
}

fn from_name<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> {
    trace(
        "from_name",
        take_until_ws.verify_map(|s| String::from_utf8(s.to_owned()).ok()),
    )
    .parse_next(input)
}

fn generic_args<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<(), E> {
    let generic_arg = (ws(1..), "--", take_until_ws);
    trace("generic_args", repeat(0.., generic_arg).map(|_: Vec<_>| ())).parse_next(input)
}

fn other_instruction<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<DockerFileInstruction, E> {
    // For now, assume any instruction can use heredoc or backslashes for continuation
    let heredoc = trace("heredoc", heredoc_start.flat_map(|eot| cut_err(heredoc_finish(eot))));
    let line_parts = (take_until_end1, comment_line_end).take();
    let lines = trace("lines", separated::<_, _, (), _, _, _, _>(1.., line_parts.void(), b"\\").take());
    let instr_name = terminated(alphanumeric0, ws(1..));

    trace(
        "other_instruction",
        (instr_name, alt((heredoc, lines))).map(|r| {
            DockerFileInstruction::Other(
                String::from_utf8_lossy(r.0).into_owned(),
                String::from_utf8_lossy(r.1).into_owned(),
            )
        }),
    )
    .parse_next(input)
}

fn heredoc_start<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> { 
    let with_redirect_out = trace("heredoc_start_with_redirect",
        (alphanumeric1, ws(0..), ">", take_till(1.., b"\r\n"), line_ending).map(|s| s.0)
    );
    let without_redirect_out = trace("heredoc_start_without_redirect",
        terminated(alphanumeric1, (ws(0..), line_ending))
    );
    let heredoc_suffix = trace("heredoc_suffix", alt((with_redirect_out, without_redirect_out))
        .map(|s| String::from_utf8_lossy(s).into_owned()));

    trace(
        "heredoc_start",
        preceded((take_until(0.., "<<"), "<<"), heredoc_suffix)
    ).parse_next(input)
}

fn heredoc_finish<'i, E>(fin: String) -> impl Parser<Stream<'i>, StreamSlice<'i>, E>
where
    E: ParserError<Stream<'i>>,
{
    let line = (till_line_ending, line_ending).take();
    let end_line = (literal_owned(fin.into_bytes()), line_ending).void();
    trace(
        "heredoc_finish",
        repeat_till::<_, _, (), _, _, _, _>(0.., line, end_line).map(|r| r.0).take()
    )
}

fn literal_owned<'i, E>(binput: Vec<u8>) -> impl Parser<Stream<'i>, StreamSlice<'i>, E>
where
    E: ParserError<Stream<'i>>,
{
    // XXX literal() does not currently work with owned types
    trace("literal_o", take(binput.len()).verify(move |b: &[u8]| b == binput))
}

fn strange_line<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<DockerFileInstruction, E> {
    // We end up here if we can't understand this line
    trace(
        "strange_line",
        (take_until_end1, line_ending)
            .map(|r| DockerFileInstruction::Strange(String::from_utf8_lossy(r.0).into_owned())),
    )
    .parse_next(input)
}

fn comment_line_end<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<(), E> {
    let ending = alt((line_ending.void(), eof.void()));
    trace(
        "comment_line_end",
        (ws(0..), opt((b"#", till_line_ending)), ending),
    )
    .value(())
    .parse_next(input)
}

fn ws<'i, E>(range: std::ops::RangeFrom<usize>) -> impl Parser<Stream<'i>, StreamSlice<'i>, E>
where
    E: ParserError<Stream<'i>>,
{
    take_while(range, b" \t")
}

fn take_until_end1<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<&'i [u8], E> {
    alt((take_till(0.., b"\r\n#"), terminated(rest, eof))).parse_next(input)
}

fn take_until_ws<'i, E: ParserError<Stream<'i>>>(
    input: &mut Stream<'i>,
) -> ModalResult<&'i [u8], E> {
    trace(
        "take_until_ws",
        alt((take_till(1.., b" \t\r\n"), terminated(rest, eof))),
    )
    .parse_next(input)
}

impl DockerFileParser {
    pub fn new() -> Self {
        Self {
            buffer: Box::new(CircularBuffer::new()),
        }
    }

    pub fn push(&mut self, data: &[u8], eof: bool) -> Vec<DockerFileInstruction> {
        let mut results = Vec::new();
        self.buffer.extend(data);

        let buffer = self.buffer.make_contiguous();
        let mut input = Stream::new(buffer);
        let mut consumed = 0;

        if eof {
            let _ = input.complete();
        }

        while !input.is_empty() {
            let start = input.checkpoint();

            match dockerfile_instructions::<ContextError>.parse_next(&mut input) {
                Ok(value) => {
                    if let Some(value) = value {
                        results.push(value);
                    }
                    consumed += input.offset_from(&start);

                    if eof && input.is_empty() {
                        break;
                    }
                }
                Err(ErrMode::Incomplete(_)) => {
                    break;
                }
                Err(e) => {
                    panic!("{e}")
                }
            }
        }

        self.buffer.consume(consumed);

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct FromInstr {
        src: String,
        name: Option<String>,
    }

    fn docker_filter_from(contents: &str) -> Vec<FromInstr> {
        let mut parser = DockerFileParser::new();
        let items = parser.push(contents.as_bytes(), true);
        items
            .into_iter()
            .filter_map(|i| match i {
                DockerFileInstruction::From { src, name } => Some(FromInstr { src, name }),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_simple() {
        assert!(docker_filter_from("").is_empty());
        assert_eq!(
            docker_filter_from("FROM src\n"),
            vec![FromInstr {
                src: "src".into(),
                name: None
            }]
        );
        assert_eq!(
            docker_filter_from("FROM src"),
            vec![FromInstr {
                src: "src".into(),
                name: None
            }]
        );
    }

    #[test]
    fn test_as() {
        assert_eq!(
            docker_filter_from("FROM src:t AS target\nRUN a\nFROM target AS target2\n"),
            vec![
                FromInstr {
                    src: "src:t".into(),
                    name: Some("target".into())
                },
                FromInstr {
                    src: "target".into(),
                    name: Some("target2".into())
                }
            ]
        );
    }

    #[test]
    fn test_partial() {
        let mut parser = DockerFileParser::new();
        let items = parser.push(b"FROM src AS target\n", false);
        assert_eq!(
            items[0],
            DockerFileInstruction::From {
                src: "src".into(),
                name: Some("target".into()),
            }
        );
    }

    #[test]
    fn test_partial2() {
        let mut parser = DockerFileParser::new();
        let items = parser.push(b"FROM src AS target\nRUN bit", false);
        assert_eq!(
            items[0],
            DockerFileInstruction::From {
                src: "src".into(),
                name: Some("target".into()),
            }
        );
    }

    #[test]
    fn test_literal_owned() {
        assert_eq!(literal_owned::<ContextError>(b"abc".into()).parse_peek(Stream::new(b"abc")), Ok((Stream::new(b""), b"abc" as &[u8])));
        assert_eq!(literal_owned::<ContextError>(b"abc".into()).parse_peek(Stream::new(b"abcd")), Ok((Stream::new(b"d"), b"abc" as &[u8])));
    }

    #[test]
    fn test_heredoc_start() {
        assert_eq!(heredoc_start::<ContextError>.parse_peek(Stream::new(b"<<ABC\n")).map(|t| t.1), Ok("ABC".into()));
    }
}
