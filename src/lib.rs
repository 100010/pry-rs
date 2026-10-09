//! An interactive REPL breakpoint for Rust, inspired by Ruby's `binding.pry`.
//!
//! Rust has no runtime reflection, so local variables cannot be captured
//! automatically. Instead, pass the variables you want to inspect to the
//! [`pry!`] macro (they must implement [`std::fmt::Debug`]):
//!
//! ```no_run
//! use pry::pry;
//!
//! let count = 42;
//! let names = vec!["alice", "bob"];
//! pry!(count, names);
//! ```
//!
//! Execution pauses and an interactive prompt opens on stdin/stdout where
//! you can inspect the captured variables (including sub-fields like
//! `user.name` or `items[0]`), view the surrounding source and a
//! backtrace, and then continue.

mod debug_value;

use std::backtrace::Backtrace;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};

/// A variable captured at the breakpoint. Built by the [`pry!`] macro;
/// you normally don't construct this yourself.
pub struct Var {
    pub name: &'static str,
    pub type_name: &'static str,
    pub compact: String,
    pub pretty: String,
}

/// Pauses execution and starts an interactive REPL on stdin/stdout.
///
/// With no arguments, only source location / backtrace inspection is
/// available. Arguments must implement `Debug` and are captured by
/// reference at the moment the macro runs:
///
/// ```no_run
/// # use pry::pry;
/// # #[derive(Debug)] struct User { name: String }
/// # let user = User { name: "alice".into() };
/// # let items = vec![1, 2, 3];
/// pry!(user, items);
/// pry!(items.len(), &items[0]); // arbitrary Debug expressions work too
/// ```
///
/// Execution resumes when you type `continue` (or `c`), or when stdin is
/// closed.
#[macro_export]
macro_rules! pry {
    () => {
        $crate::start(file!(), line!(), ::std::vec::Vec::new())
    };
    ($($var:expr),+ $(,)?) => {
        $crate::start(file!(), line!(), ::std::vec![
            $(
                $crate::Var {
                    name: stringify!($var),
                    type_name: ::std::any::type_name_of_val(&$var),
                    compact: format!("{:?}", $var),
                    pretty: format!("{:#?}", $var),
                }
            ),+
        ])
    };
}

/// Line input abstraction: a readline editor when interactive, plain
/// buffered reads otherwise (pipes, tests).
trait LineReader {
    /// Reads one line. Returns `None` on EOF. The prompt is either
    /// rendered by the reader itself (readline) or written to `out`.
    fn read_line(&mut self, prompt: &str, out: &mut dyn Write) -> Option<String>;
}

struct PipeReader<R>(R);

impl<R: BufRead> LineReader for PipeReader<R> {
    fn read_line(&mut self, prompt: &str, out: &mut dyn Write) -> Option<String> {
        let _ = write!(out, "{prompt}");
        let _ = out.flush();
        let mut buf = String::new();
        match self.0.read_line(&mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(buf),
        }
    }
}

#[cfg(feature = "readline")]
struct TtyReader(rustyline::DefaultEditor);

#[cfg(feature = "readline")]
impl LineReader for TtyReader {
    fn read_line(&mut self, prompt: &str, _out: &mut dyn Write) -> Option<String> {
        match self.0.readline(prompt) {
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = self.0.add_history_entry(line.as_str());
                }
                Some(line)
            }
            // Ctrl-C cancels the current line, like in Ruby's pry.
            Err(rustyline::error::ReadlineError::Interrupted) => Some(String::new()),
            Err(_) => None, // Ctrl-D / EOF
        }
    }
}

/// Implementation detail of [`pry!`]. Not part of the public API surface
/// beyond being callable from the macro expansion.
#[doc(hidden)]
pub fn start(file: &str, line: u32, vars: Vec<Var>) {
    let color = use_color();
    #[cfg(feature = "readline")]
    if io::stdin().is_terminal() {
        if let Ok(editor) = rustyline::DefaultEditor::new() {
            run_with(TtyReader(editor), file, line, vars, color);
            return;
        }
    }
    let stdin = io::stdin();
    run_with(PipeReader(stdin.lock()), file, line, vars, color);
}

fn run_with<L: LineReader>(input: L, file: &str, line: u32, vars: Vec<Var>, color: bool) {
    let mut session = Session {
        file,
        line: line as usize,
        vars,
        color,
        input,
        output: io::stdout(),
    };
    session.run();
}

struct Session<'a, L, W> {
    file: &'a str,
    line: usize,
    vars: Vec<Var>,
    color: bool,
    input: L,
    output: W,
}

const RED: &str = "31";
const GREEN: &str = "32";
const YELLOW: &str = "33";
const CYAN: &str = "36";
const GRAY: &str = "90";
const BOLD: &str = "1";

impl<L: LineReader, W: Write> Session<'_, L, W> {
    fn run(&mut self) {
        let location = self.paint(&format!("From: {}:{}", self.file, self.line), YELLOW);
        let _ = writeln!(self.output, "\n{location}");
        self.show_source(5);
        let hint = self.paint("Type \"help\" for commands, \"continue\" to resume.", GRAY);
        let _ = writeln!(self.output, "{hint}");

        loop {
            let Some(input) = self.input.read_line("pry> ", &mut self.output) else {
                let _ = writeln!(self.output);
                return;
            };
            let input = input.trim();
            if input.is_empty() {
                continue;
            }
            match input {
                "continue" | "c" => return,
                "exit" | "quit" => std::process::exit(0),
                "help" | "h" => self.print_help(),
                "ls" => self.list_vars(),
                "whereami" => {
                    let location =
                        self.paint(&format!("From: {}:{}", self.file, self.line), YELLOW);
                    let _ = writeln!(self.output, "{location}");
                    self.show_source(5);
                }
                "bt" => {
                    let _ = writeln!(self.output, "{}", Backtrace::force_capture());
                }
                _ => self.inspect(input),
            }
        }
    }

    fn inspect(&mut self, input: &str) {
        // "p <expr>" prints the pretty ({:#?}) form.
        let (expr, pretty) = match input.strip_prefix("p ") {
            Some(rest) => (rest.trim(), true),
            None => (input, false),
        };

        // Exact match against a captured variable.
        if let Some(var) = self.vars.iter().find(|v| v.name == expr) {
            let value = if pretty {
                var.pretty.clone()
            } else {
                var.compact.clone()
            };
            let ty = format!("({})", var.type_name);
            self.print_result(&ty, &value);
            return;
        }

        // Path access into a captured variable: user.name, items[0], ...
        match self.resolve_path(expr, pretty) {
            Ok(Some(value)) => self.print_result("", &value),
            Ok(None) => {
                let err = self.paint("error:", RED);
                let _ = writeln!(
                    self.output,
                    "{err} unknown variable or command {expr:?} (try \"ls\" or \"help\")"
                );
            }
            Err(msg) => {
                let err = self.paint("error:", RED);
                let _ = writeln!(self.output, "{err} {msg}");
            }
        }
    }

    /// Ok(Some(rendered)) on success, Ok(None) if `expr` is not a path or
    /// its base is unknown, Err for navigation errors worth reporting.
    fn resolve_path(&self, expr: &str, pretty: bool) -> Result<Option<String>, String> {
        let Some((base, segs)) = debug_value::parse_path(expr) else {
            return Ok(None);
        };
        let Some(var) = self.vars.iter().find(|v| v.name == base) else {
            return Ok(None);
        };
        let tree = debug_value::parse(&var.compact).ok_or_else(|| {
            format!("could not parse the Debug output of {base} (custom Debug impl?)")
        })?;
        let node = debug_value::navigate(&tree, &segs)?;
        Ok(Some(if pretty {
            node.render_pretty()
        } else {
            node.render_compact()
        }))
    }

    fn print_result(&mut self, ty: &str, value: &str) {
        let arrow = self.paint("=>", GREEN);
        if ty.is_empty() {
            let _ = writeln!(self.output, "{arrow} {value}");
        } else {
            let ty = self.paint(ty, GRAY);
            let _ = writeln!(self.output, "{arrow} {ty} {value}");
        }
    }

    fn list_vars(&mut self) {
        if self.vars.is_empty() {
            let msg = self.paint("(no variables; pass them via pry!(a, b, ...))", GRAY);
            let _ = writeln!(self.output, "{msg}");
            return;
        }
        for i in 0..self.vars.len() {
            let name = self.paint(self.vars[i].name, CYAN);
            let ty = self.paint(self.vars[i].type_name, GRAY);
            let _ = writeln!(self.output, "{name} {ty} = {}", self.vars[i].compact);
        }
    }

    fn print_help(&mut self) {
        let _ = writeln!(
            self.output,
            "Commands:\n\
             \x20 ls              list captured variables\n\
             \x20 <name>          print a variable ({{:?}})\n\
             \x20 <name>.field    access sub-fields: user.name, items[0].id, map[\"key\"]\n\
             \x20 p <name>        pretty-print ({{:#?}}), also works with paths\n\
             \x20 whereami        show source around the breakpoint\n\
             \x20 bt              show a backtrace\n\
             \x20 continue, c     resume execution\n\
             \x20 exit, quit      terminate the program\n\
             \x20 help, h         show this help"
        );
    }

    fn show_source(&mut self, context: usize) {
        let Ok(content) = fs::read_to_string(self.file) else {
            return;
        };
        let lines: Vec<&str> = content.lines().collect();
        let start = self.line.saturating_sub(context).max(1);
        let end = (self.line + context).min(lines.len());
        let width = end.to_string().len();
        for n in start..=end {
            let marker = if n == self.line { "=>" } else { "  " };
            let text = format!("{:>width$}: {}", n, lines[n - 1], width = width);
            let text = if n == self.line {
                self.paint(&text, BOLD)
            } else {
                text
            };
            let marker = self.paint(marker, GREEN);
            let _ = writeln!(self.output, " {marker} {text}");
        }
    }

    fn paint(&self, text: &str, code: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

fn use_color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_session(input: &str, vars: Vec<Var>) -> String {
        let mut output = Vec::new();
        let mut session = Session {
            file: "src/lib.rs",
            line: 1,
            vars,
            color: false,
            input: PipeReader(input.as_bytes()),
            output: &mut output,
        };
        session.run();
        String::from_utf8(output).unwrap()
    }

    fn var(name: &'static str, value: impl std::fmt::Debug) -> Var {
        Var {
            name,
            type_name: std::any::type_name_of_val(&value),
            compact: format!("{value:?}"),
            pretty: format!("{value:#?}"),
        }
    }

    #[derive(Debug)]
    #[allow(dead_code)]
    struct Item {
        name: &'static str,
        price: u32,
    }

    #[derive(Debug)]
    #[allow(dead_code)]
    struct Order {
        id: u64,
        items: Vec<Item>,
    }

    fn sample_order() -> Order {
        Order {
            id: 1001,
            items: vec![
                Item {
                    name: "keyboard",
                    price: 12000,
                },
                Item {
                    name: "mouse",
                    price: 4500,
                },
            ],
        }
    }

    #[test]
    fn ls_lists_variables_with_types() {
        let out = run_session("ls\nc\n", vec![var("count", 42i32), var("name", "alice")]);
        assert!(out.contains("count i32 = 42"), "output: {out}");
        assert!(out.contains("name &str = \"alice\""), "output: {out}");
    }

    #[test]
    fn ls_with_no_variables() {
        let out = run_session("ls\nc\n", vec![]);
        assert!(out.contains("no variables"), "output: {out}");
    }

    #[test]
    fn prints_variable_value() {
        let out = run_session("count\nc\n", vec![var("count", 42i32)]);
        assert!(out.contains("=> (i32) 42"), "output: {out}");
    }

    #[test]
    fn pretty_prints_variable() {
        let out = run_session("p items\nc\n", vec![var("items", vec![1, 2])]);
        assert!(out.contains("1,\n    2,\n]"), "output: {out}");
    }

    #[test]
    fn field_access_on_struct() {
        let out = run_session("order.items\nc\n", vec![var("order", sample_order())]);
        assert!(
            out.contains("=> [Item { name: \"keyboard\", price: 12000 }"),
            "output: {out}"
        );
    }

    #[test]
    fn nested_field_and_index_access() {
        let out = run_session(
            "order.items[1].name\norder.id\nc\n",
            vec![var("order", sample_order())],
        );
        assert!(out.contains("=> \"mouse\""), "output: {out}");
        assert!(out.contains("=> 1001"), "output: {out}");
    }

    #[test]
    fn pretty_path_access() {
        let out = run_session("p order.items[0]\nc\n", vec![var("order", sample_order())]);
        assert!(
            out.contains("Item {\n    name: \"keyboard\",\n    price: 12000,\n}"),
            "output: {out}"
        );
    }

    #[test]
    fn path_errors_are_reported() {
        let out = run_session(
            "order.nope\norder.items[9]\nc\n",
            vec![var("order", sample_order())],
        );
        assert!(out.contains("no field \"nope\""), "output: {out}");
        assert!(out.contains("out of range"), "output: {out}");
    }

    #[test]
    fn reference_debug_output_is_navigable() {
        // pry!(&order) captures "&Order" type but identical Debug text.
        let order = sample_order();
        let out = run_session("order.id\nc\n", vec![var("order", &order)]);
        assert!(out.contains("=> 1001"), "output: {out}");
    }

    #[test]
    fn unknown_name_reports_error() {
        let out = run_session("nope\nc\n", vec![]);
        assert!(out.contains("unknown variable or command"), "output: {out}");
    }

    #[test]
    fn shows_source_context() {
        let out = run_session("c\n", vec![]);
        // Line 1 of this very file is the crate doc comment.
        assert!(out.contains("From: src/lib.rs:1"), "output: {out}");
        assert!(out.contains("1: //!"), "output: {out}");
    }

    #[test]
    fn eof_resumes_execution() {
        let out = run_session("", vec![]);
        assert!(out.contains("pry>"), "output: {out}");
    }

    #[test]
    fn help_lists_commands() {
        let out = run_session("help\nc\n", vec![]);
        assert!(out.contains("whereami"), "output: {out}");
        assert!(out.contains("continue"), "output: {out}");
    }

    #[test]
    fn macro_captures_expressions() {
        // The macro accepts plain vars and expressions; here we expand the
        // capture list the same way the macro does (start() would block on
        // stdin in a test environment).
        let items = vec![1, 2, 3];
        let vars = vec![
            var("items", &items),
            Var {
                name: stringify!(items.len()),
                type_name: std::any::type_name_of_val(&items.len()),
                compact: format!("{:?}", items.len()),
                pretty: format!("{:#?}", items.len()),
            },
        ];
        // "items.len()" resolves as an exact captured name, not a path.
        let out = run_session("items.len()\nitems[2]\nc\n", vars);
        assert!(out.contains("=> (usize) 3"), "output: {out}");
        assert!(out.contains("=> 3"), "output: {out}");
    }
}
