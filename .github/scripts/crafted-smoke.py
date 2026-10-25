#!/usr/bin/env python3
"""Bounded desktop smoke tests for privately supplied, unmodified guest IPAs.

A smoke pass means the guest reached CPU execution and did not produce a
recognised emulator panic during the observation window. It does NOT establish
visual correctness, gameplay, input, audio, or iOS-host compatibility.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import time

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
    parser.add_argument("--screenshots", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=30)
    args = parser.parse_args()

    if not args.emulator.is_file():
        parser.error("Emulator executable is missing")
    games = sorted(args.games.rglob("*.ipa"))
    if not games:
        parser.error("No IPA files found in the private test archive")

    args.logs.mkdir(parents=True, exist_ok=True)
    args.screenshots.mkdir(parents=True, exist_ok=True)
    results = []
    for index, game in enumerate(games, 1):
        print(f"Smoke-testing {game.name} ({index}/{len(games)})", flush=True)
        cmd = [str(args.emulator.resolve()), str(game.resolve()), "--no-error-popup"]
        env = os.environ.copy()
        env.setdefault("SDL_AUDIODRIVER", "dummy")
        timed_out = False
        screenshot = args.screenshots / f"game-{index}-main-menu.png"
        singleplayer_screenshot = args.screenshots / f"game-{index}-singleplayer.png"
        create_world_screenshot = args.screenshots / f"game-{index}-create-world.png"
        screenshot_error = None
        log_path = args.logs / f"game-{index}.log"
        try:
            with log_path.open("w", encoding="utf-8") as log_file:
                with subprocess.Popen(
                    cmd,
                    cwd=Path.cwd(),
                    stdout=log_file,
                    stderr=subprocess.STDOUT,
                    env=env,
                    start_new_session=True,
                ) as proc:
                    capture_after = min(45, max(1, args.seconds * 2 // 3))
                    try:
                        proc.wait(timeout=capture_after)
                    except subprocess.TimeoutExpired:
                        try:
                            capture = subprocess.run(
                                ["import", "-window", "root", str(screenshot)],
                                capture_output=True,
                                text=True,
                                timeout=15,
                            )
                        except subprocess.TimeoutExpired:
                            screenshot_error = "Timed out capturing rendered screen"
                            capture = None
                        if capture is not None and capture.returncode != 0:
                            screenshot_error = capture.stderr.strip() or "Image capture failed"
                        if screenshot_error is None and screenshot.is_file():
                            try:
                                geometry = subprocess.check_output(
                                    ["xdotool", "getdisplaygeometry"],
                                    text=True,
                                    timeout=5,
                                )
                                screen_width, screen_height = map(int, geometry.split())
                                subprocess.run(
                                    [
                                        "xdotool",
                                        "mousemove",
                                        "--sync",
                                        str(screen_width // 2),
                                        str(screen_height // 2 - 4),
                                        "click",
                                        "1",
                                    ],
                                    check=True,
                                    timeout=5,
                                )
                                time.sleep(3)
                                capture = subprocess.run(
                                    [
                                        "import",
                                        "-window",
                                        "root",
                                        str(singleplayer_screenshot),
                                    ],
                                    capture_output=True,
                                    text=True,
                                    timeout=15,
                                )
                                if capture.returncode != 0:
                                    screenshot_error = (
                                        capture.stderr.strip()
                                        or "Singleplayer screen capture failed"
                                    )
                                elif hashlib.sha256(screenshot.read_bytes()).digest() == hashlib.sha256(
                                    singleplayer_screenshot.read_bytes()
                                ).digest():
                                    screenshot_error = "Singleplayer tap did not change the rendered screen"
                                if screenshot_error is None:
                                    subprocess.run(
                                        [
                                            "xdotool",
                                            "mousemove",
                                            "--sync",
                                            str(screen_width // 2 + screen_width // 30),
                                            str(screen_height * 2 // 3),
                                            "click",
                                            "1",
                                        ],
                                        check=True,
                                        timeout=5,
                                    )
                                    time.sleep(3)
                                    capture = subprocess.run(
                                        [
                                            "import",
                                            "-window",
                                            "root",
                                            str(create_world_screenshot),
                                        ],
                                        capture_output=True,
                                        text=True,
                                        timeout=15,
                                    )
                                    if capture.returncode != 0:
                                        screenshot_error = (
                                            capture.stderr.strip()
                                            or "Create New World screen capture failed"
                                        )
                                    elif hashlib.sha256(
                                        singleplayer_screenshot.read_bytes()
                                    ).digest() == hashlib.sha256(
                                        create_world_screenshot.read_bytes()
                                    ).digest():
                                        screenshot_error = (
                                            "Create New World tap did not change the rendered screen"
                                        )
                            except (OSError, subprocess.SubprocessError, ValueError) as exc:
                                screenshot_error = f"Unable to exercise Singleplayer control: {exc}"
                        try:
                            proc.wait(timeout=max(1, args.seconds - capture_after - 3))
                        except subprocess.TimeoutExpired:
                            timed_out = True
                            proc.kill()
                            proc.wait()
                    return_code = proc.returncode
        except OSError as exc:
            with log_path.open("a", encoding="utf-8") as log_file:
                log_file.write(f"Unable to launch emulator: {exc}\n")
            return_code = -1

        output = log_path.read_text(encoding="utf-8", errors="replace")
        has_cpu = CPU_STARTED in output
        panic = PANIC.search(output)
        if panic:
            status, reason = "FAIL", f"Emulator panic: {panic.group(0)}"
        elif not has_cpu:
            status, reason = "BLOCKED", "Guest CPU execution not reached; check graphics/host setup"
        elif return_code not in (0, None) and not timed_out:
            status, reason = "FAIL", f"Process exited with code {return_code}"
        elif (
            screenshot_error
            or not screenshot.is_file()
            or not singleplayer_screenshot.is_file()
            or not create_world_screenshot.is_file()
        ):
            status, reason = "FAIL", screenshot_error or "No rendered screen was captured"
        else:
            status, reason = "SMOKE PASS", (
                "No detected panic during observation window (process ended normally)"
                if not timed_out
                else "No detected panic during observation window (process was time-limited)"
            )
        print(
            f"  {status}: {reason}; diagnostic log: {log_path}; "
            f"screens: {screenshot}, {singleplayer_screenshot}",
            flush=True,
        )
        results.append((game.name, status, reason))

    summary = "## Crafted private desktop smoke tests\n\n"
    summary += (
        "This checks that the main menu renders, Singleplayer opens world selection, "
        "and Create New World changes the screen. It does not establish world "
        "loading or full gameplay.\n\n"
    )
    summary += "| Game | Result | Note |\n| --- | --- | --- |\n"
    for game, status, reason in results:
        summary += f"| {game.replace('|', ' ')} | {status} | {reason} |\n"
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as file:
            file.write(summary)

    return 1 if any(status != "SMOKE PASS" for _, status, _ in results) else 0


if __name__ == "__main__":
    sys.exit(main())
