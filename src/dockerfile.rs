use std::io::BufRead;

use circular_buffer::CircularBuffer;
use winnow::{
    ascii::{newline, till_line_ending, Caseless}, combinator::{alt, delimited, opt, repeat, seq}, error::{ContextError, ErrMode, ParserError}, prelude::*, stream::Offset as _, token::{take_till, take_while}, Partial
};

pub struct DockerFileParser {
    buffer: Box<CircularBuffer<4096, u8>>, 
}

type Stream<'i> = Partial<&'i [u8]>;

#[derive(Debug, Clone)]
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
        delimited(ws, single_instruction, comment_line_end).map(Some),
        comment_line_end.value(None),
        strange_line.map(Some),
    )).parse_next(input)
}

fn single_instruction<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    alt((
        from_instruction,
        other_instruction,
    )).parse_next(input)
}

fn from_instruction<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    let mut from_as = opt(seq!(_: ws, _: Caseless("as"), _: ws, from_name));
    seq!(_: Caseless("from"), _: generic_args, _: ws, from_image, from_as).map(|r| {
        DockerFileInstruction::From {
            src: r.0,
            name: r.1.map(|s| s.0)
        }
    }).parse_next(input)
}

fn from_image<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> {
    take_until_ws.verify_map(|s| String::from_utf8(s.to_owned()).ok()).parse_next(input)
}

fn from_name<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<String, E> {
    take_until_ws.verify_map(|s| String::from_utf8(s.to_owned()).ok()).parse_next(input)
}

fn generic_args<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<(), E> {
    let generic_arg = (ws, "--", take_until_ws);
    repeat(0.., generic_arg).map(|_: Vec<_>| ()).parse_next(input)
}

fn other_instruction<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    // TODO: Make sure we handle heredoc, backslash, etc, correctly
    (take_until_end1, comment_line_end).map(|r| {
        DockerFileInstruction::Other(String::from_utf8_lossy(r.0).into_owned())
    }).parse_next(input)
}

fn strange_line<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<DockerFileInstruction, E> {
    // We end up here if we can't understand this line
    (take_until_end1, newline).map(|r| {
        DockerFileInstruction::Strange(String::from_utf8_lossy(r.0).into_owned())
    }).parse_next(input)
}

fn comment_line_end<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<(), E> {
    (ws, opt((b"#", till_line_ending)), newline).value(()).parse_next(input)
}

fn ws<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<&'i [u8], E> {
    take_while(0.., b" \t").parse_next(input)
}

fn take_until_end1<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<&'i [u8], E> {
    take_till(1.., b"\r\n#").parse_next(input)
}

fn take_until_ws<'i, E: ParserError<Stream<'i>>>(input: &mut Stream<'i>) -> ModalResult<&'i [u8], E> {
    take_till(0.., b" \t\r\n").parse_next(input)
}

impl DockerFileParser {
    pub fn new() -> Self {
        Self {
            buffer: Box::new(CircularBuffer::new()),
        }
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<DockerFileInstruction> {
        let mut results = Vec::new();
        self.buffer.extend(data);
        
        loop {
            let buffer = self.buffer.make_contiguous();
            let mut input = Stream::new(buffer);
            let start = input.checkpoint();

            match dockerfile_instructions::<ContextError>.parse_next(&mut input) {
                Ok(value) => {
                    if let Some(value) = value {
                        results.push(value);
                    }
                    let consumed = input.offset_from(&start);
                    self.buffer.consume(consumed);
                },
                Err(ErrMode::Incomplete(_)) => {
                    break;
                },
                Err(e) => {
                    panic!("{e}")
                }
            }
        }

        results
    }
}
