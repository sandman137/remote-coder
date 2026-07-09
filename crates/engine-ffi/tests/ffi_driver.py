#!/usr/bin/env python3
"""FFI boundary proof on Linux (DESIGN.md §10.1 layer 5, §12 Phase 8).

Loads the desktop-built libhelm_engine.so through the generated UniFFI
Python (ctypes) bindings and drives the engine end to end — connect,
enumerate, snapshot, send keys, press an adapter button, and receive events
via BOTH poll_events and the EngineListener callback. This is the runnable
on-this-box analogue of the design's JVM/Kotlin test: it proves the exact
FFI surface Android will call, with no emulator and no JVM toolchain.

Usage (wired by scripts/run-ffi-test.sh and `just ffi-jvm`):
  PYTHONPATH=.dev/ffi/python python3 crates/engine-ffi/tests/ffi_driver.py <tmux-socket>
"""
import os
import subprocess
import sys
import time
import threading

import helm_engine as h


def log(msg):
    print(f"[ffi-driver] {msg}", flush=True)


def row_text(grid, row):
    # Reconstruct a row straight from the flat FFI cell array — proves the
    # Vec<CellFfi> marshalled correctly (records carry data, not methods).
    start = row * grid.cols
    chars = []
    for c in grid.cells[start:start + grid.cols]:
        if not c.wide_continuation:
            chars.append(c.ch)
    return "".join(chars).rstrip()


def grid_text(grid):
    return "\n".join(row_text(grid, r) for r in range(grid.rows))


def wait_snapshot_contains(engine, pane, needle, timeout=15):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        grid = engine.snapshot(pane, 0)
        last = grid_text(grid)
        if needle in last:
            return grid
        time.sleep(0.15)
    raise AssertionError(f"timed out waiting for {needle!r}; last:\n{last}")


class CollectingListener(h.EngineListener):
    """Push-model listener (§7.4) — proves the callback interface links and
    fires from the Rust event-forwarding thread into Python."""

    def __init__(self):
        self.events = []
        self.lock = threading.Lock()

    def on_event(self, event):
        with self.lock:
            self.events.append(event)

    def kinds(self):
        with self.lock:
            return [type(e).__name__ for e in self.events]


def main():
    socket = sys.argv[1]
    fixtures = os.path.join(os.path.dirname(__file__), "..", "..", "..", "fixtures", "agents")
    fixtures = os.path.abspath(fixtures)

    def tmux(*args):
        subprocess.run(["tmux", "-L", socket, "-f", "/dev/null", *args], check=True)

    # ---- smoke: the .so loaded and links ----
    ver = h.engine_version()
    log(f"engine_version() = {ver}")
    assert ver.count(".") == 2, ver

    bits = h.cell_attr_bits()
    assert bits.bold == 1 and bits.reverse == 16, bits

    try:
        tmux("new-session", "-d", "-s", "agents", "-x", "90", "-y", "28",
             os.path.join(fixtures, "fake-yn.sh"))

        # ---- connect over the FFI boundary ----
        engine = h.HelmEngine.connect(h.ConnConfigFfi.LOCAL(socket=socket))
        log("connected")

        listener = CollectingListener()
        engine.set_listener(listener)

        # ---- enumerate ----
        sessions = engine.list_sessions()
        assert any(s.name == "agents" for s in sessions), [s.name for s in sessions]
        panes = engine.list_panes("agents")
        assert len(panes) == 1, panes
        pane = panes[0].id
        assert panes[0].width == 90 and panes[0].height == 28
        log(f"pane {pane} {panes[0].width}x{panes[0].height} cmd={panes[0].current_command}")

        # ---- snapshot: flat grid marshalling ----
        grid = wait_snapshot_contains(engine, pane, "Proceed? (y/n)")
        assert grid.cols == 90 and grid.rows == 28
        assert any("Proceed? (y/n)" in row_text(grid, r) for r in range(grid.rows))
        assert grid.cursor is not None
        # The "●" working line is cyan (indexed 6) — prove color crossed FFI.
        colored = False
        for r in range(grid.rows):
            start = r * grid.cols
            for c in grid.cells[start:start + grid.cols]:
                if isinstance(c.fg, h.ColorFfi.INDEXED) and c.fg.index in (2, 6):
                    colored = True
        assert colored, "expected an indexed-color cell to cross the FFI"
        log("snapshot OK (grid + color marshalled)")

        # ---- streaming events via poll + callback ----
        engine.attach(pane, 90, 28)
        deadline = time.time() + 15
        saw_grid_poll = False
        while time.time() < deadline and not saw_grid_poll:
            for ev in engine.poll_events():
                if isinstance(ev, h.EngineEventFfi.GRID):
                    saw_grid_poll = True
            time.sleep(0.1)
        assert saw_grid_poll, "no GRID event via poll_events"
        log("poll_events delivered a GRID event")

        # ---- send keys + adapter button ----
        engine.send_keys(pane, "y")
        wait_snapshot_contains(engine, pane, "proceeding")
        log("send_keys advanced the agent")

        wait_snapshot_contains(engine, pane, "Proceed? (y/n)")
        engine.press_button(pane, "No")
        wait_snapshot_contains(engine, pane, "step aborted.")
        log("press_button('No') worked through the FFI")

        # Attention/metadata should also have reached the callback listener.
        deadline = time.time() + 10
        while time.time() < deadline:
            kinds = listener.kinds()
            if any("GRID" in k for k in kinds):
                break
            time.sleep(0.1)
        kinds = listener.kinds()
        assert any("GRID" in k for k in kinds), f"listener saw no grid events: {set(kinds)}"
        log(f"listener received {len(kinds)} events incl. grid ({len(set(kinds))} kinds)")

        print("[ffi-driver] ALL FFI CHECKS PASSED", flush=True)
    finally:
        subprocess.run(["tmux", "-L", socket, "kill-server"],
                       stderr=subprocess.DEVNULL, check=False)


if __name__ == "__main__":
    main()
