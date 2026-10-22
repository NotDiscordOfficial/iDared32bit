#!/usr/bin/env python3
"""Bounded desktop smoke tests for privately supplied, unmodified guest IPAs.

A smoke pass means the guest reached CPU execution and did not produce a
recognised emulator panic during the observation window. It does NOT establish
visual correctness, gameplay, input, audio, or iOS-host compatibility.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

PANIC = re.compile(
    r"(?:Panic at |Call to unimplemented function|thread '.*' panicked at|"
    r"assertion failed:|stack backtrace:)",
    re.IGNORECASE,
)
CPU_STARTED = "CPU emulation begins now."


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--emulator", type=Path, required=True)
    parser.add_argument("--games", type=Path, required=True)
    parser.add_argument("--logs", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=30)
    args = parser.parse_args()

    if not args.emulator.is_file():
        parser.error("Emulator executable is missing")
    games = sorted(args.games.rglob("*.ipa"))
    if not games:
        parser.error("No IPA files found in the private test archive")

    args.logs.mkdir(parents=True, exist_ok=True)
    results = []
    for index, game in enumerate(games, 1):
        print(f"Smoke-testing {game.name} ({index}/{len(games)})", flush=True)
        cmd = [str(args.emulator.resolve()), str(game.resolve()), "--no-error-popup"]
        env = os.environ.copy()
        env.setdefault("SDL_AUDIODRIVER", "dummy")
        timed_out = False
        try:
            with subprocess.Popen(
                cmd,
                cwd=Path.cwd(),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                env=env,
                start_new_session=True,
            ) as proc:
                try:
                    output, _ = proc.communicate(timeout=args.seconds)
                except subprocess.TimeoutExpired:
                    timed_out = True
                    proc.kill()
                    output, _ = proc.communicate()
                return_code = proc.returncode
        except OSError as exc:
            output, return_code = f"Unable to launch emulator: {exc}\n", -1

        log_path = args.logs / f"game-{index}.log"
        log_path.write_text(output, encoding="utf-8")
        has_cpu = CPU_STARTED in output
        panic = PANIC.search(output)
        if panic:
            status, reason = "FAIL", f"Emulator panic: {panic.group(0)}"
        elif not has_cpu:
            status, reason = "BLOCKED", "Guest CPU execution not reached; check macOS graphics/host setup"
        elif return_code not in (0, None) and not timed_out:
            status, reason = "FAIL", f"Process exited with code {return_code}"
        else:
            status, reason = "SMOKE PASS", (
                "No detected panic during observation window (process ended normally)"
                if not timed_out
                else "No detected panic during observation window (process was time-limited)"
            )
        print(f"  {status}: {reason}; diagnostic log: {log_path}", flush=True)
        results.append((game.name, status, reason))

    summary = "## Crafted private desktop smoke tests\n\n"
    summary += "This only checks startup, not actual game playability on iOS.\n\n"
    summary += "| Game | Result | Note |\n| --- | --- | --- |\n"
    for game, status, reason in results:
        summary += f"| {game.replace('|', ' ')} | {status} | {reason} |\n"
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as file:
            file.write(summary)

    return 1 if any(status != "SMOKE PASS" for _, status, _ in results) else 0


if __name__ == "__main__":
    sys.exit(main())
