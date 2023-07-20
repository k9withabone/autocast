# autocast

A tool to help automate the creation of terminal demos. Automatically generate an asciicast file for use with [asciinema](https://asciinema.org/).

[![demo](./demo.gif)](https://asciinema.org/a/597756)

Demo created with autocast, see [demo.yaml](./demo.yaml). The demo is also viewable on [asciinema](https://asciinema.org/a/597756).

## Features

- Generates asciicast files from the settings and instructions in an input YAML file.
- Fast, run time is dependent upon the run time of the shell commands, with minimal overhead.
- Use bash, python, or a custom shell.
- Customize the output's prompt and secondary prompt, separate from the shell's.
- Use hidden commands for automated setup and cleanup.

## Installation

- Download a prebuilt binary from [releases](https://github.com/k9withabone/autocast/releases).
- Use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) to get a prebuilt binary with `cargo binstall autocast`.
- Build and install with `cargo install autocast`.

## Usage

### CLI

```
$ autocast -h

Automate terminal demos

Usage: autocast [OPTIONS] <IN_FILE> <OUT_FILE>

Arguments:
  <IN_FILE>   Input file to create the asciicast file with
  <OUT_FILE>  Output asciicast file

Options:
      --width <WIDTH>
          Terminal width
      --height <HEIGHT>
          Terminal height
  -t, --title <TITLE>
          Title of the asciicast
      --shell <SHELL>
          Shell to use for running commands [default: bash] [possible values: bash, python]
  -e, --environment <NAME=VALUE>
          Environment variables to use in the shell process
      --environment-capture <ENV_VAR>
          Environment variables to capture [default: TERM] [aliases: env-cap]
  -d, --type-speed <TYPE_SPEED>
          Default time between key presses when writing commands [default: 100ms] [aliases: delay]
      --prompt <PROMPT>
          The shell prompt to use in the asciicast output [default: "$ "]
      --secondary-prompt <SECONDARY_PROMPT>
          The shell secondary prompt to use in the asciicast output [default: "> "]
      --timeout <TIMEOUT>
          Maximum amount of time to let a shell command run before returning with an error [default: 30s]
      --overwrite
          Overwrite output file if it already exists
  -h, --help
          Print help (see more with '--help')
  -V, --version
          Print version
```

Use `autocast --help` to see a more in-depth explanation of the CLI arguments. Also see their corresponding settings in [full-example.yaml](./full-example.yaml).

Non-default CLI arguments will override settings specified in the input YAML file.

### Input YAML File

For examples, see [example.yaml](./example.yaml) and [demo.yaml](./demo.yaml). For an in-depth explanation of all configuration values, see [full-example.yaml](./full-example.yaml).

Instruction Kinds:

- Command
    - Normal shell command or control code.
    - Can be a single line or split across multiple lines.
    - Waits until the shell prompt is displayed to ensure the command has completed.
    - Optionally hidden from asciicast output.
- Interactive
    - Starts an interactive shell command like an editor or TUI app.
    - Requires a list of keys that are used to control the started command.
        - Keys are input in real time (including any waits) while output is continuously captured.
    - After all the keys are fed to the command, it must exit, and the shell returned to the prompt.
    - Like the normal shell command, it waits until the shell prompt is displayed before running the next instruction.
- Wait
    - Adds time between the output of the last instruction and the start of the next.
    - This time is only added in the asciicast output and does not increase the run time of autocast.
- Marker
    - Adds a marker to the asciicast output.
    - Markers are chapters that show in the asciinema web player.
- Clear
    - Adds output events to the asciicast output that will clear the terminal.

## Contribution

Contributions/suggestions are very welcome and appreciated!
Feel free to create an [issue](https://github.com/k9withabone/autocast/issues), [discussion](https://github.com/k9withabone/autocast/discussions), or [pull request](https://github.com/k9withabone/autocast/pulls).
Especially in need of default configurations for other shells (zsh, fish, etc.) as I have no experience with shells other than bash.

## Inspiration

- [asciinema](https://asciinema.org/)
- [VHS](https://github.com/charmbracelet/vhs)
- [asciinema_automation](https://github.com/PierreMarchand20/asciinema_automation)
- [expectrl](https://crates.io/crates/expectrl)
    - Used as a dependency, but some of the top level functionality was reimplemented (specifically [`Session`](https://docs.rs/expectrl/latest/expectrl/session/struct.Session.html) and [`ReplSession`](https://docs.rs/expectrl/latest/expectrl/repl/struct.ReplSession.html) in `ShellSession`) for autocast's needs.

## License

Autocast is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0.en.html) or later, see the [license](./LICENSE) file for details.
