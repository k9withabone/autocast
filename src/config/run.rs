use std::{io, iter, mem, thread, time::Duration};

use color_eyre::eyre::Context;
use itertools::Itertools;

use crate::asciicast::Event;

use super::{spawn::ShellSession, Command, Instruction, Key};

pub(super) fn instructions<'a, T: FromIterator<Event>>(
    instructions: impl IntoIterator<Item = &'a Instruction>,
    prompt: &str,
    secondary_prompt: &str,
    type_speed: Duration,
    line_split: &str,
    shell_session: &mut ShellSession,
) -> color_eyre::Result<T> {
    instructions
        .into_iter()
        .scan(Duration::ZERO, |wait_time, instruction| {
            let events = instruction.run(
                prompt,
                secondary_prompt,
                type_speed,
                line_split,
                shell_session,
            );
            let events = match events {
                Ok(events) => events,
                Err(error) => return Some(Err(error)),
            };
            if let Events::Wait(wait) = events {
                *wait_time += wait;
            }
            let mut events = events.peekable();
            if !wait_time.is_zero() {
                if let Some(event) = events.peek_mut() {
                    event.time += mem::take(wait_time);
                }
            }
            Some(Ok(events))
        })
        .process_results(|events| {
            iter::once(Event::output(Duration::ZERO, String::from(prompt)))
                .chain(events.flatten())
                .chain(iter::once(Event::outputln(type_speed)))
                .scan(Duration::ZERO, |time, mut event| {
                    event.time += *time;
                    *time = event.time;
                    Some(event)
                })
                .collect()
        })
}

impl Instruction {
    fn run<'a>(
        &'a self,
        prompt: &'a str,
        secondary_prompt: &'a str,
        default_type_speed: Duration,
        line_split: &'a str,
        shell_session: &mut ShellSession,
    ) -> color_eyre::Result<Events<impl Iterator<Item = Event> + 'a, impl Iterator<Item = Event>>>
    {
        match self {
            Self::Command {
                command,
                hidden,
                type_speed,
            } => {
                command
                    .send(shell_session)
                    .wrap_err("could not send command to shell")?;
                let mut output = shell_session
                    .read_until_prompt()
                    .wrap_err("could not read shell output")?;

                if *hidden {
                    return Ok(Events::None);
                }

                output.push(shell_session.new_event(String::from(prompt)));
                let type_speed = type_speed.unwrap_or(default_type_speed);
                let events = command
                    .events(type_speed, secondary_prompt, line_split)
                    .chain(output);

                Ok(Events::Command(events))
            }
            Self::Interactive {
                command,
                keys,
                type_speed,
            } => {
                command
                    .send(shell_session)
                    .wrap_err("could not send command to shell")?;

                let type_speed = type_speed.map_or(default_type_speed, Into::into);
                let mut output = keys_to_events(keys, type_speed, shell_session)?;

                output.push(shell_session.new_event(String::from(prompt)));
                let events = command
                    .events(type_speed, secondary_prompt, line_split)
                    .chain(output);

                Ok(Events::Command(events))
            }
            Self::Wait(duration) => Ok(Events::Wait(*duration)),
            Self::Marker(data) => Ok(Events::once(Event::marker(Duration::ZERO, data.clone()))),
            Self::Clear => {
                let clear =
                    Event::output(default_type_speed, String::from("\r\x1b[H\x1b[2J\x1b[3J"));
                let prompt = Event::output(default_type_speed, String::from(prompt));
                Ok(Events::Clear([clear, prompt].into_iter()))
            }
        }
    }
}

fn keys_to_events(
    keys: &[Key],
    type_speed: Duration,
    shell_session: &mut ShellSession,
) -> color_eyre::Result<Vec<Event>> {
    let (first, mut prompt_seen) = shell_session.read().wrap_err("could not read from shell")?;

    let output = keys
        .iter()
        .map_while(|key| {
            if prompt_seen {
                return None;
            }
            let result = key
                .send(shell_session)
                .wrap_err("could not send key to shell")
                .and_then(|_| {
                    thread::sleep(type_speed);
                    let (event, prompt) =
                        shell_session.read().wrap_err("could not read from shell")?;
                    prompt_seen = prompt;
                    Ok(event)
                });
            Some(result)
        })
        .filter_map(Result::transpose);
    thread::sleep(type_speed);
    let mut output: Vec<_> = first.map(Ok).into_iter().chain(output).try_collect()?;

    if !prompt_seen {
        let events = shell_session
            .read_until_prompt()
            .wrap_err("could not read prompt from shell")?;
        output.extend(events);
    }

    Ok(output)
}

#[derive(Debug, Clone)]
enum Events<Co, Cl> {
    Command(Co),
    Clear(Cl),
    Once(iter::Once<Event>),
    Wait(Duration),
    None,
}

impl<Co, Cl> Iterator for Events<Co, Cl>
where
    Co: Iterator<Item = Event>,
    Cl: Iterator<Item = Event>,
{
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Command(iter) => iter.next(),
            Self::Clear(iter) => iter.next(),
            Self::Once(iter) => iter.next(),
            Self::Wait(_) | Self::None => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Command(iter) => iter.size_hint(),
            Self::Clear(iter) => iter.size_hint(),
            Self::Once(iter) => iter.size_hint(),
            Self::Wait(_) | Self::None => (0, Some(0)),
        }
    }
}

impl<Co, Cl> Events<Co, Cl> {
    fn once(event: Event) -> Self {
        Self::Once(iter::once(event))
    }
}

impl Command {
    fn send(&self, shell_session: &mut ShellSession) -> io::Result<()> {
        shell_session.reset();
        match self {
            Self::SingleLine(line) => shell_session.send_line(line),
            Self::MultiLine(lines) => shell_session.send_line(&lines.join(" ")),
            Self::Control(control) => shell_session.send(control),
        }
    }

    fn events<'a>(
        &'a self,
        type_speed: Duration,
        secondary_prompt: &'a str,
        line_split: &'a str,
    ) -> impl Iterator<Item = Event> + 'a {
        match self {
            Self::SingleLine(line) => {
                CommandEvents::SingleLine(type_line(type_speed, line.chars()))
            }
            Self::MultiLine(lines) => {
                let num_lines = lines.len();
                let iter = lines.iter().enumerate().flat_map(move |(line_num, line)| {
                    let secondary_prompt = (line_num != 0)
                        .then(|| Event::output(type_speed, String::from(secondary_prompt)));

                    let line_split = (line_num + 1 < num_lines)
                        .then_some(line_split.chars())
                        .into_iter()
                        .flatten();

                    secondary_prompt
                        .into_iter()
                        .chain(type_line(type_speed, line.chars().chain(line_split)))
                });
                CommandEvents::MultiLine(iter)
            }
            Self::Control(control) => {
                let control: &str = control.as_ref();
                CommandEvents::Control(type_line(type_speed, control.chars()))
            }
        }
    }
}

fn type_line(
    type_speed: Duration,
    line: impl IntoIterator<Item = char>,
) -> impl Iterator<Item = Event> {
    line.into_iter()
        .map(move |char| Event::output(type_speed, String::from(char)))
        .chain(iter::once(Event::outputln(type_speed)))
}

#[derive(Debug, Clone)]
enum CommandEvents<S, M, C> {
    SingleLine(S),
    MultiLine(M),
    Control(C),
}

impl<S, M, C> Iterator for CommandEvents<S, M, C>
where
    S: Iterator<Item = Event>,
    M: Iterator<Item = Event>,
    C: Iterator<Item = Event>,
{
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::SingleLine(iter) => iter.next(),
            Self::MultiLine(iter) => iter.next(),
            Self::Control(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::SingleLine(iter) => iter.size_hint(),
            Self::MultiLine(iter) => iter.size_hint(),
            Self::Control(iter) => iter.size_hint(),
        }
    }
}

impl Key {
    fn send(&self, shell_session: &mut ShellSession) -> io::Result<()> {
        match self {
            Self::Char(char) => shell_session.send([*char as u8]),
            Self::Control(control) => shell_session.send(control),
            Self::Wait(duration) => {
                thread::sleep(*duration);
                Ok(())
            }
        }
    }
}
