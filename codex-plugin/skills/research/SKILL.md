---
name: research
description: Run an Axiom Chronos research question in the user's project.
---

# Research

Invoke the installed `axiom research` from the user's project working directory. Forward the user's flags and text in their original order and argument boundaries, without rewriting or interpreting them. Use an argument array or correctly quote each argument for the shell; never interpolate raw user text into a shell command. Preserve `--project` and `--max-turns` when supplied; let the CLI handle its existing calling convention.

Let the Rust host run its own workflow: separate Astra core/research threads, Terra dispatcher/workers, Sol assembly, and Luna docs/images/memory. Do not create native collaboration subagents or duplicate that workflow.

Use bounded command waits and continue polling a running process until it exits. Stdin EOF is allowed; where a running PTY is available, send user interjections as text followed by Enter. Report any resume instructions printed by the host. Exit code 0 alone does not establish completion: consult the host's goal/log status and report the actual outcome. Do not request manual approvals already authorized.
