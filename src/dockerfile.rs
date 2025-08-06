use std::{io::BufRead};

use circular_buffer::CircularBuffer;
use winnow::{
    ascii::{newline, till_line_ending, Caseless}, combinator::{alt, delimited, eof, opt, repeat, seq, terminated, trace}, error::{ContextError, ErrMode, ParserError}, prelude::*, stream::Offset as _, token::{rest, take_till, take_while}, Partial
};

pub struct DockerFileParser {
    buffer: Box<CircularBuffer<4096, u8>>, 
}

type Stream<'i> = Partial<&'i [u8]>;
type StreamSlice<'i> = <Stream<'i> as winnow::stream::Stream>::Slice;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DockerFileInstruction {
    From {
        src: String,
        name: Option<String>,
    },
    Other(String),
    Strange(String),
}

fn dockerfile_instructions<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<Option<DockerFileInstruction>, E> {
    alt((
        delimited(ws(0..), single_instruction, comment_line_end).map(Some),
        comment_line_end.value(None),
        strange_line.map(Some),
    )).parse_next(input)
}

fn single_instruction<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    trace("single_instruction", alt((
        from_instruction,
        other_instruction,
    ))).parse_next(input)
}

fn from_instruction<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    let mut from_as = opt(seq!(_: ws(1..), _: Caseless("as"), _: ws(1..), from_name));
    seq!(_: Caseless("from"), _: generic_args, _: ws(1..), from_image, from_as).map(|r| {
        DockerFileInstruction::From {
            src: r.0,
            name: r.1.map(|s| s.0)
        }
    }).parse_next(input)
}

fn from_image<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> {
    trace("from_image", take_until_ws.verify_map(|s| String::from_utf8(s.to_owned()).ok())).parse_next(input)
}

fn from_name<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> {
    trace("from_name", take_until_ws.verify_map(|s| String::from_utf8(s.to_owned()).ok())).parse_next(input)
}

fn generic_args<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<(), E> {
    let generic_arg = (ws(1..), "--", take_until_ws);
    trace("generic_args", repeat(0.., generic_arg).map(|_: Vec<_>| ())).parse_next(input)
}

fn other_instruction<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    // TODO: Make sure we handle heredoc, backslash, etc, correctly
    trace("other_instruction", (take_until_end1, comment_line_end).map(|r| {
        DockerFileInstruction::Other(String::from_utf8_lossy(r.0).into_owned())
    })).parse_next(input)
}

fn strange_line<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    // We end up here if we can't understand this line
    trace("strange_line", (take_until_end1, newline).map(|r| {
        DockerFileInstruction::Strange(String::from_utf8_lossy(r.0).into_owned())
    })).parse_next(input)
}

fn comment_line_end<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<(), E> {
    let ending = alt((newline, eof.value(0 as char)));
    trace("comment_line_end", (
        ws(0..),
        opt((b"#", till_line_ending)),
        ending
    )).value(()).parse_next(input)
}

fn ws<'i, E>(range: std::ops::RangeFrom<usize>) -> impl Parser<Stream<'i>, StreamSlice<'i>, E>
where
    E: ParserError<Stream<'i>>
{
    take_while(range, b" \t")
}

fn take_until_end1<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<&'i [u8], E> {
    alt((
        take_till(0.., b"\r\n#"),
        terminated(rest, eof)
    )).parse_next(input)
}

fn take_until_ws<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<&'i [u8], E> {
    trace("take_until_ws", alt((
        take_till(1.., b" \t\r\n"),
        terminated(rest, eof)
    ))).parse_next(input)
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
                },
                Err(ErrMode::Incomplete(_)) => {
                    break;
                },
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
        items.into_iter().filter_map(|i|
            match i {
                DockerFileInstruction::From { src, name } => Some(FromInstr { src, name}),
                _ => None,
            }
        ).collect()
    }

    #[test]
    fn test_simple() {
        assert!(docker_filter_from("").is_empty());
        assert_eq!(docker_filter_from("FROM src\n"), vec![
            FromInstr { src: "src".into(), name: None }
        ]);
        assert_eq!(docker_filter_from("FROM src"), vec![
            FromInstr { src: "src".into(), name: None }
        ]);
    }

    #[test]
    fn test_as() {
        assert_eq!(docker_filter_from("FROM src:t AS target\nRUN a\nFROM target AS target2\n"), vec![
            FromInstr { src: "src:t".into(), name: Some("target".into()) },
            FromInstr { src: "target".into(), name: Some("target2".into()) }
        ]);
    }

    #[test]
    fn test_partial() {
        let mut parser = DockerFileParser::new();
        let items = parser.push(b"FROM src AS target\n", false);
        assert_eq!(items[0], DockerFileInstruction::From {
            src: "src".into(),
            name: Some("target".into()),
        });
    }

    #[test]
    fn test_partial2() {
        let mut parser = DockerFileParser::new();
        let items = parser.push(b"FROM src AS target\nRUN bit", false);
        assert_eq!(items[0], DockerFileInstruction::From {
            src: "src".into(),
            name: Some("target".into()),
        });
    }
}
