use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use radar_store::refresh::{
    run_refresh, LiveExecutor, RefreshContext, StepStatus, REQUIRED_REPO,
};

fn main() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    println!("working directory={}", cwd.display());
    let cwd_s = cwd.to_string_lossy();
    if !cwd_s.eq_ignore_ascii_case(REQUIRED_REPO) {
        eprintln!("STOP: working directory is not {REQUIRED_REPO}");
        return ExitCode::from(2);
    }
    if cwd_s.to_ascii_lowercase().contains("karl-intel-desktop")
        || cwd_s.to_ascii_lowercase().contains("mvp_hub")
    {
        eprintln!("STOP: this is Karl-Intel-Desktop or MVP_Hub");
        return ExitCode::from(2);
    }
    println!("repo confirmed DrData_ICP_Lead_radar (not Karl-Intel-Desktop, not MVP_Hub)");
    println!("refresh_all is a sequencer. It does not delete or overwrite existing loads.");
    println!("detect_improvement_zero_to_value is excluded until it compares the latest two comparable loads.");
    let _ = io::stdout().flush();

    let ctx = match RefreshContext::for_live(&cwd) {
        Ok(ctx) => ctx,
        Err(err) => {
            eprintln!("failed to prepare refresh context: {err}");
            return ExitCode::from(1);
        }
    };
    if let Some(start) = &ctx.accela_poll_start {
        println!("accela next poll start={start} (from newest stored record_date + 1 day)");
    } else {
        println!("accela next poll start cannot be computed; that step will skip");
    }
    let _ = io::stdout().flush();

    let mut executor = LiveExecutor;
    let mut out = io::stdout();
    let report = match run_refresh(&mut out, &ctx, &mut executor) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("refresh_all failed to write output: {err}");
            return ExitCode::from(1);
        }
    };

    println!(
        "refresh_all done succeeded={} skipped={} failed={}",
        report.succeeded_count(),
        report.skipped_count(),
        report.failed_count()
    );
    if report.steps.iter().any(|s| s.status == StepStatus::Failed) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
