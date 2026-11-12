//! The command line. A process with arguments is a command; with none, it
//! prints what it can do.

fn main() {
    std::process::exit(xproxy::cli::main());
}
