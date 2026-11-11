use super::{Instance, Ledger, Tally, Warning};
use crate::session::render::comma;
use std::fmt::Write;

const INSTANCE_LINES: usize = 10;
const WARNING_LINES: usize = 15;

fn dollars(usd: Option<f64>) -> String {
    usd.map_or_else(|| "unpriced".to_owned(), |v| format!("${v:.2}"))
}

fn row(out: &mut String, agent: &str, model: &str, t: &Tally) {
    let _ = writeln!(
        out,
        "{agent:<26} {model:<20} {:>6} {:>10} {:>13} {:>13} {:>14} {:>11} {:>10} {:>10} {:>5}",
        comma(t.calls),
        comma(t.input),
        comma(t.cache_write),
        comma(t.cache_write_1h),
        comma(t.cache_read),
        comma(t.output),
        dollars(t.usd),
        comma(t.peak_context),
        comma(t.gap_rewrites),
    );
}

fn instance_line(out: &mut String, i: &Instance) {
    let _ = writeln!(
        out,
        "  {:>10}  {:<26} {:<20} {}{}",
        comma(i.tally.peak_context),
        i.agent_type,
        i.id,
        i.description.as_deref().unwrap_or(""),
        if i.compactions > 0 {
            format!(" ({} compacted)", i.compactions)
        } else {
            String::new()
        }
    );
}

fn who(agent_type: &str, instance: &str) -> String {
    if agent_type == instance {
        agent_type.to_owned()
    } else {
        format!("{agent_type} {instance}")
    }
}

fn warning_line(w: &Warning) -> String {
    match w {
        Warning::ContextHigh {
            agent_type,
            instance,
            model,
            peak_context,
        } => format!(
            "{}: context reached {} tokens on {model}",
            who(agent_type, instance),
            comma(*peak_context)
        ),
        Warning::CacheRewritesAfterGap {
            agent_type,
            instance,
            count,
            tokens,
            usd,
        } => format!(
            "{}: {count} large cache write(s) after a long gap, {} tokens, ${usd:.2}",
            who(agent_type, instance),
            comma(*tokens)
        ),
        Warning::LargeFileRead {
            agent_type,
            instance,
            path,
            reads,
            largest_bytes,
        } => format!(
            "{}: whole read of {path} ({reads}x, largest {} bytes)",
            who(agent_type, instance),
            comma(*largest_bytes)
        ),
        Warning::CopiedHistoryNotSeparable { file, zeroed_calls } => format!(
            "{file}: holds history copied from an earlier session that this format cannot separate ({zeroed_calls} zeroed calls); totals may include it"
        ),
    }
}

pub fn render(ledger: &Ledger) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Session {}", ledger.session);
    if let Some(cwd) = &ledger.cwd {
        let _ = writeln!(out, "Project {cwd}");
    }
    let _ = writeln!(
        out,
        "Source  {} ({} subagent transcripts)\n",
        ledger.source, ledger.subagent_files
    );
    let _ = writeln!(
        out,
        "{:<26} {:<20} {:>6} {:>10} {:>13} {:>13} {:>14} {:>11} {:>10} {:>10} {:>5}",
        "agent",
        "model",
        "calls",
        "input",
        "cache write",
        "of it 1h",
        "cache read",
        "output",
        "USD",
        "peak ctx",
        "gaps"
    );
    for r in &ledger.by_agent {
        row(&mut out, &r.agent_type, &r.model, &r.tally);
    }
    row(&mut out, "total", "", &ledger.totals);
    if ledger.totals.unpriced_calls > 0 {
        let _ = writeln!(
            out,
            "\n{} calls have no known price; priced calls come to ${:.2}.",
            comma(ledger.totals.unpriced_calls),
            ledger.totals.priced_usd
        );
    }

    let _ = writeln!(out, "\nLargest agent contexts (tokens):");
    for i in ledger.instances.iter().take(INSTANCE_LINES) {
        instance_line(&mut out, i);
    }
    if ledger.instances.len() > INSTANCE_LINES {
        let _ = writeln!(
            out,
            "  and {} more",
            ledger.instances.len() - INSTANCE_LINES
        );
    }

    if ledger.totals.gap_rewrites > 0 {
        let _ = writeln!(
            out,
            "\nLarge cache writes after a gap over {} minutes: {} ({} tokens, ${:.2} of writes)",
            ledger.thresholds.cache_gap_seconds / 60,
            comma(ledger.totals.gap_rewrites),
            comma(ledger.totals.gap_rewrite_tokens),
            ledger.totals.gap_rewrite_usd
        );
    }

    if !ledger.codex.is_empty() || ledger.codex_runs_without_usage > 0 {
        let _ = writeln!(out, "\nCodex:");
        for c in &ledger.codex {
            let _ = writeln!(
                out,
                "  {}: {} runs, {} input ({} cached), {} output ({} reasoning)",
                c.model,
                comma(c.runs),
                comma(c.tokens.input),
                comma(c.tokens.cached_input),
                comma(c.tokens.output),
                comma(c.tokens.reasoning_output)
            );
        }
        if ledger.codex_runs_without_usage > 0 {
            let _ = writeln!(
                out,
                "  {} runs recorded no usage",
                comma(ledger.codex_runs_without_usage)
            );
        }
    }

    if !ledger.warnings.is_empty() {
        let _ = writeln!(out, "\nWarnings:");
        for w in ledger.warnings.iter().take(WARNING_LINES) {
            let _ = writeln!(out, "  {}", warning_line(w));
        }
        if ledger.warnings.len() > WARNING_LINES {
            let _ = writeln!(out, "  and {} more", ledger.warnings.len() - WARNING_LINES);
        }
    }

    let _ = writeln!(out);
    if let Some(usd) = ledger.claude_code_usd {
        let _ = writeln!(out, "Claude Code's own last saved total: ${usd:.2}");
    }
    if ledger.compactions > 0 {
        let _ = writeln!(
            out,
            "Compactions: {}. Transcripts do not record their usage, so USD is a lower bound.",
            comma(ledger.compactions)
        );
    }
    if ledger.copied_calls > 0 {
        let _ = writeln!(
            out,
            "{} calls copied from an earlier session are excluded.",
            comma(ledger.copied_calls)
        );
    }
    if ledger.skipped_lines > 0 {
        let _ = writeln!(
            out,
            "{} unreadable lines were skipped (a truncated last line is normal during a run).",
            comma(ledger.skipped_lines)
        );
    }
    if ledger.output_tokens_partial {
        let _ = writeln!(
            out,
            "No transcript was found: output tokens come from the stream log and are lower bounds."
        );
    }
    let _ = writeln!(
        out,
        "Prices as of {} ({}).",
        ledger.prices.as_of, ledger.prices.source
    );
    out
}
