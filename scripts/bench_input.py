#!/usr/bin/env python3
"""Measure PTY echo latency through isolated real griz-core instances (not GPU scanout).

Example: python3 scripts/bench_input.py --binary target/debug/griz-core \
    --size 160x48 --samples 60 --output artifacts/input-latency.json

Measures complete socket-frame arrival separately from Python JSON decoding. Every
sample waits for a changed terminal cursor, not just the input acknowledgement.
Only the temporary service and its shell are stopped on completion.
"""
import argparse
import json
import os
from pathlib import Path
import select
import socket
import statistics
import struct
import subprocess
import tempfile
import time


def send(stream, value):
    data = json.dumps(value, separators=(",", ":")).encode()
    stream.sendall(struct.pack(">I", len(data)) + data)


def receive(stream):
    def exact(size):
        data = bytearray()
        while len(data) < size:
            chunk = stream.recv(size - len(data))
            if not chunk:
                raise RuntimeError("service disconnected")
            data.extend(chunk)
        return data

    size = struct.unpack(">I", exact(4))[0]
    data = exact(size)
    arrived = time.perf_counter_ns()
    event = json.loads(data)
    decoded = time.perf_counter_ns()
    if event["type"] == "error":
        raise RuntimeError(event["message"])
    return event, arrived, decoded, size + 4


def summary(values):
    values = sorted(values)
    return {
        "median": round(statistics.median(values), 3),
        "p95": round(values[min(len(values) - 1, int(len(values) * 0.95))], 3),
        "min": round(values[0], 3),
        "max": round(values[-1], 3),
    }


def measure(binary, cols, rows, samples, pause, preview_cycle=False):
    with tempfile.TemporaryDirectory(prefix="griz-input-") as directory:
        endpoint = str(Path(directory) / "core.sock")
        env = {**os.environ, "GRIZ_SOCKET": endpoint, "PS1": "bench$ ", "ENV": ""}
        with open(Path(directory) / "core.log", "w+") as log:
            process = subprocess.Popen(
                [str(binary), "--shell", "/bin/sh", "--cwd", directory],
                env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
            )
            stream = socket.socket(socket.AF_UNIX)
            stream.settimeout(5)
            try:
                deadline = time.monotonic() + 5
                while True:
                    try:
                        stream.connect(endpoint)
                        break
                    except (FileNotFoundError, ConnectionRefusedError):
                        if process.poll() is not None or time.monotonic() > deadline:
                            log.seek(0)
                            raise RuntimeError(log.read())
                        time.sleep(0.01)
                send(stream, {"type": "attach", "cols": cols, "rows": rows})
                event, *_ = receive(stream)
                state = event["state"]
                # Wait for the actual shell prompt and drain startup pictures.
                while select.select([stream], [], [], 0.1)[0]:
                    event, *_ = receive(stream)
                    if event["type"] == "view":
                        state = event["state"]
                terminal_id = state["active_terminal"]
                serial = 0
                if preview_cycle:
                    for target in [{"type": "workspace", "workspace_id": state["active_workspace"]}, None]:
                        serial += 1
                        send(stream, {"type": "command", "serial": serial,
                                      "command": {"type": "set_preview", "target": target}})
                        while True:
                            event, *_ = receive(stream)
                            if event["type"] == "view" and event["state"]["ack"] >= serial:
                                state = event["state"]
                                break
                    if state.get("preview") is not None:
                        raise RuntimeError("preview remained subscribed after closing")
                records = []
                for index in range(samples + 8):
                    previous = state["terminal"]["screen"]
                    old_cursor = (previous["cursor_x"], previous["cursor_y"])
                    serial += 1
                    command = {"type": "input", "terminal_id": terminal_id,
                               "data": [ord("x") if index % 2 == 0 else 127]}
                    started = time.perf_counter_ns()
                    send(stream, {"type": "command", "serial": serial, "command": command})
                    frames = total_bytes = 0
                    first_ack = None
                    while True:
                        event, arrived, decoded, size = receive(stream)
                        total_bytes += size
                        if event["type"] != "view":
                            continue
                        frames += 1
                        state = event["state"]
                        if state["ack"] < serial:
                            continue
                        if first_ack is None:
                            first_ack = (arrived - started) / 1e6
                        screen = state["terminal"]["screen"]
                        if (screen["cursor_x"], screen["cursor_y"]) != old_cursor:
                            break
                    if index >= 8:
                        records.append({
                            "echo_arrival_ms": (arrived - started) / 1e6,
                            "echo_decoded_ms": (decoded - started) / 1e6,
                            "first_ack_ms": first_ack,
                            "frames": frames, "bytes": total_bytes,
                        })
                    time.sleep(pause)
                return {
                    "binary": str(binary), "grid": f"{cols}x{rows}", "samples": samples,
                    "pause_ms": pause * 1000, "preview_cycle": preview_cycle,
                    **{field: summary([r[field] for r in records]) for field in records[0]},
                    "raw": records,
                }
            finally:
                stream.close()
                try:
                    with socket.socket(socket.AF_UNIX) as shutdown:
                        shutdown.settimeout(1)
                        shutdown.connect(endpoint)
                        send(shutdown, {"type": "shutdown"})
                    process.wait(timeout=3)
                except (OSError, subprocess.TimeoutExpired):
                    process.kill()
                    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, action="append", required=True)
    parser.add_argument("--size", action="append", help="COLSxROWS (repeatable)")
    parser.add_argument("--samples", type=int, default=60)
    parser.add_argument("--pause", type=float, default=0.02, help="seconds between samples")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--preview-cycle", action="store_true", help="subscribe then unsubscribe before typing (requires preview-capable core)")
    args = parser.parse_args()
    if args.samples < 1 or args.pause < 0:
        parser.error("samples must be positive and pause nonnegative")
    results = []
    for size in args.size or ["80x24", "160x48"]:
        cols, rows = map(int, size.split("x"))
        for binary in args.binary:
            result = measure(binary.resolve(), cols, rows, args.samples, args.pause, args.preview_cycle)
            results.append(result)
            print(json.dumps({k: v for k, v in result.items() if k != "raw"}), flush=True)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
