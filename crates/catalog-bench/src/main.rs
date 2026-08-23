use std::path::PathBuf;

use catalog_bench::{BenchmarkConfig, run_benchmark};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (assets, output) = parse_arguments()?;
    let report = run_benchmark(BenchmarkConfig {
        assets,
        batch_size: 500,
    })?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

fn parse_arguments() -> Result<(u64, PathBuf), String> {
    let mut assets = None;
    let mut output = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--assets" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--assets requires a value".to_owned())?;
                assets = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| "--assets must be an unsigned integer".to_owned())?,
                );
            }
            "--output" => {
                output = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--output requires a path".to_owned())?,
                ));
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }
    Ok((
        assets.ok_or_else(|| "--assets is required".to_owned())?,
        output.ok_or_else(|| "--output is required".to_owned())?,
    ))
}
