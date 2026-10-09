# pry-rs

An interactive REPL breakpoint for Rust, inspired by Ruby's `binding.pry`.

<img width="920" height="640" alt="demo" src="https://github.com/user-attachments/assets/537bdbad-8f46-49e7-ae1e-477f822f9c95" />


Drop a `pry!()` into your code and execution pauses with an interactive
prompt where you can inspect variables, view the surrounding source, and
print a backtrace — then continue running.

The prompt supports line editing and history (arrow keys, Ctrl-A/E/H,
etc.) via [rustyline]. The only dependency is rustyline, and even that can
be disabled with `default-features = false` for a std-only build.

[rustyline]: https://github.com/kkawakam/rustyline

## Why not exactly like `binding.pry`?

Ruby is fully dynamic, so `binding.pry` can see every local variable
automatically. Rust is compiled and has no runtime reflection, so you tell
`pry!` which variables (or expressions) to capture. Anything that
implements `Debug` works.

## Installation

```toml
[dependencies]
pry-rs = { git = "https://github.com/100010/pry-rs" }
```

## Usage

```rust
use pry::pry;

#[derive(Debug)]
struct User {
    name: String,
    age: u32,
}

fn main() {
    let user = User { name: "alice".into(), age: 30 };
    let scores = [88, 92, 75];

    pry!(user, scores, scores.len()); // ← execution pauses here

    println!("resumed!");
}
```

Running this opens a prompt:

```text
From: src/main.rs:13
    ...
 => 13:     pry!(user, scores, scores.len());
    ...
Type "help" for commands, "continue" to resume.
pry> ls
user User = User { name: "alice", age: 30 }
scores [i32; 3] = [88, 92, 75]
scores.len() usize = 3
pry> p user
=> (User) User {
    name: "alice",
    age: 30,
}
pry> user.name
=> "alice"
pry> scores[1]
=> 92
pry> c
resumed!
```

## Commands

| Command        | Description                              |
| -------------- | ---------------------------------------- |
| `ls`           | list captured variables                  |
| `<name>`       | print a variable (`{:?}`)                |
| `<name>.field` | access sub-fields: `user.name`, `items[0].id`, `map["key"]` |
| `p <name>`     | pretty-print (`{:#?}`), also works with paths |
| `whereami`     | show source around the breakpoint        |
| `bt`           | show a backtrace                         |
| `continue`, `c`| resume execution                         |
| `exit`, `quit` | terminate the program                    |
| `help`, `h`    | show this help                           |

## Notes

- Values are captured (as `Debug` output) at the moment `pry!` runs; the
  REPL shows a snapshot, it cannot mutate your program's state.
- Sub-field access (`user.name`, `items[0]`) works by parsing the captured
  `Debug` text. Output from `#[derive(Debug)]` parses reliably; exotic
  hand-written `Debug` impls may not.
- For full backtraces, run with `RUST_BACKTRACE=1`.
- Colors are disabled automatically when stdout is not a terminal or when
  `NO_COLOR` is set.
- Requires Rust 1.76+ (`std::any::type_name_of_val`).

## Try the example

```sh
cargo run --example basic
```

## License

MIT
