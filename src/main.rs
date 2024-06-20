use clap::Parser;

mod engine;
use engine::PaymentsEngine;

#[derive(Debug, Parser)]
struct Args {
    #[clap(index = 1, help = "Path to CSV file containing transactions")]
    input_file: String,
}

fn main() {
    let args = Args::parse();
    let mut engine = PaymentsEngine::new(args.input_file);
    engine.run();
    engine.print_accounts(&mut std::io::stdout());
}
