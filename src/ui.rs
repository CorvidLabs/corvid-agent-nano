//! Terminal UI helpers — colored output for the CAN CLI.

use colored::Colorize;

/// Print a section header (bold cyan).
pub fn header(text: &str) {
    println!("{}", text.bold().cyan());
}

/// Print a success message (green checkmark + text).
pub fn success(text: &str) {
    println!("{} {}", "✓".green().bold(), text.green());
}

/// Print an error message (red cross + text).
pub fn error(text: &str) {
    eprintln!("{} {}", "✗".red().bold(), text.red());
}

/// Print a warning message (yellow).
pub fn warn(text: &str) {
    println!("{} {}", "⚠".yellow().bold(), text.yellow());
}

/// Print an info label-value pair (bold label + value).
pub fn field(label: &str, value: &str) {
    println!("  {:<12} {}", label.bold(), value);
}

/// Print a dimmed separator line.
pub fn separator(width: usize) {
    println!("{}", "─".repeat(width).dimmed());
}

/// Format a table header row (bold + underlined).
pub fn table_header(text: &str) {
    println!("{}", text.bold());
}

/// Print the CAN banner.
pub fn banner() {
    println!(
        "{}",
        r#"
   ██████╗ █████╗ ███╗   ██╗
  ██╔════╝██╔══██╗████╗  ██║
  ██║     ███████║██╔██╗ ██║
  ██║     ██╔══██║██║╚██╗██║
  ╚██████╗██║  ██║██║ ╚████║
   ╚═════╝╚═╝  ╚═╝╚═╝  ╚═══╝"#
            .cyan()
            .bold()
    );
    println!("  {}\n", "Corvid Agent — AlgoChat on Algorand".dimmed());
}

/// Direction arrow for inbox messages.
pub fn dir_arrow(direction: &str) -> String {
    if direction == "sent" {
        ">>>".blue().bold().to_string()
    } else {
        "<<<".green().bold().to_string()
    }
}

/// Format a balance value.
pub fn balance(algo: f64) -> String {
    if algo < 0.1 {
        format!("{:.6} ALGO", algo).red().to_string()
    } else {
        format!("{:.6} ALGO", algo).green().to_string()
    }
}
