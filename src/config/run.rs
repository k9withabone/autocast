use std::{
    io, iter,
    time::{Duration, Instant},
};

use color_eyre::eyre::Context;
use indicatif::{MultiProgress, ProgressDrawTarget, ProgressIterator, ProgressStyle};
use itertools::Itertools;

use crate::asciicast::Event;

use super::{spawn::ShellSession, Command, Instruction, Key};

pub(super) fn instructions<'a, I>(
    instructions: I,
    prompt: &str,
    secondary_prompt: &str,
    type_speed: Duration,
    line_split: &str,
    shell_session: &mut ShellSession,
) -> color_eyre::Result<Vec<Event>>
where
    I: IntoIterator<Item = &'a Instruction>,
    I::IntoIter: ExactSizeIterator,
{
    let mut instructions = instructions
        .into_iter()
        .progress()
        .with_style(progress_style())
        .with_prefix("Instructions");

    let multi_progress = MultiProgress::with_draw_target(ProgressDrawTarget::stdout());
    instructions.progress = multi_progress.add(instructions.progress);
    instructions
        .progress
        .enable_steady_tick(Duration::from_secs(1));

    instructions
        .enumerate()
        .map(|(num, instruction)| {
            instruction
                .run(
                    prompt,
                    secondary_prompt,
                    type_speed,
                    line_split,
                    shell_session,
                    &multi_progress,
                )
                .wrap_err_with(|| format!("error running instruction {num}"))
        })
        .process_results(|events| {
            let mut wait_time = Duration::ZERO;
            let events = events.flat_map(|mut events| {
                if let Events::Wait(wait) = events {
                    wait_time += wait;
                }
                let first = events.next().map(|mut event| {
                    event.time += wait_time;
                    wait_time = Duration::ZERO;
                    event
                });
                first.into_iter().chain(events)
            });

            let mut events = iter::once(Event::output(Duration::ZERO, String::from(prompt)))
                .chain(events)
                .chain(iter::once(Event::outputln(type_speed)))
                .scan(Duration::ZERO, |time, mut event| {
                    event.time += *time;
                    *time = event.time;
                    Some(event)
                })
                .collect_vec();
            if let Some(last) = events.last_mut() {
                last.time += wait_time;
            }
            events
        })
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:>12}: {wide_bar} {pos:>3}/{len:3} [{elapsed}]")
        .expect("invalid progress style template")
}

impl Instruction {
    fn run<'a>(
        &'a self,
        prompt: &'a str,
        secondary_prompt: &'a str,
        default_type_speed: Duration,
        line_split: &'a str,
        shell_session: &mut ShellSession,
        multi_progress: &MultiProgress,
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
                let mut output = keys_to_events(keys, type_speed, shell_session, multi_progress)?;

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
    multi_progress: &MultiProgress,
) -> color_eyre::Result<Vec<Event>> {
    let mut keys = keys
        .iter()
        .progress()
        .with_style(progress_style())
        .with_prefix("Keys");
    keys.progress = multi_progress.add(keys.progress);

    let mut events = Vec::new();
    let mut next = Instant::now() + type_speed;
    loop {
        let (event, prompt) = shell_session
            .read()
            .wrap_err("error reading shell output")?;
        events.extend(event);
        if prompt {
            return Ok(events);
        }
        keys.progress.tick();
        if Instant::now() >= next {
            if let Some(key) = keys.next() {
                key.send(shell_session).wrap_err("error sending key")?;
                if let Key::Wait(wait) = key {
                    next += *wait;
                }
                next += type_speed;
            } else {
                keys.progress.finish_and_clear();
                multi_progress.remove(&keys.progress);
                events.extend(
                    shell_session
                        .read_until_prompt()
                        .wrap_err("could not detect prompt")?,
                );
                return Ok(events);
            }
        }
    }
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

            Self::String(str) => str
                .chars()
                .map(|char| shell_session.send([char as u8]))
                .collect::<io::Result<()>>(),
            Self::Control(control) => shell_session.send(control),
            Self::Wait(_) => Ok(()),
        }
    }
}
