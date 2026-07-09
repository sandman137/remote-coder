// Kotlin FFI driver (DESIGN.md §12 Phase 8). The JNA-based UniFFI Kotlin
// bindings this exercises are the exact ones the Android app loads. This box
// has no kotlinc/gradle, so the runnable Phase-8 proof is the Python driver
// (scripts/run-ffi-test.sh); this file keeps the Kotlin path honest and
// ready for any host that has the JVM/Kotlin toolchain:
//
//   kotlinc -cp jna.jar remotecoder_engine.kt FfiDriver.kt -include-runtime -d ffi.jar
//   java -Djna.library.path=target/debug -cp ffi.jar:jna.jar FfiDriverKt <socket>
//
// It mirrors ffi_driver.py step for step.

import uniffi.remotecoder_engine.*

fun rowText(grid: GridSnapshotFfi, row: Int): String {
    val start = row * grid.cols.toInt()
    val sb = StringBuilder()
    for (i in start until start + grid.cols.toInt()) {
        val c = grid.cells[i]
        if (!c.wideContinuation) sb.append(c.ch)
    }
    return sb.toString().trimEnd()
}

fun gridText(grid: GridSnapshotFfi): String =
    (0 until grid.rows.toInt()).joinToString("\n") { rowText(grid, it) }

fun waitSnapshot(engine: RemoteCoderEngine, pane: String, needle: String, timeoutMs: Long = 15000): GridSnapshotFfi {
    val deadline = System.currentTimeMillis() + timeoutMs
    var last = ""
    while (System.currentTimeMillis() < deadline) {
        val grid = engine.snapshot(pane, 0u)
        last = gridText(grid)
        if (last.contains(needle)) return grid
        Thread.sleep(150)
    }
    throw AssertionError("timed out waiting for '$needle'; last:\n$last")
}

class CollectingListener : EngineListener {
    val events = mutableListOf<EngineEventFfi>()
    @Synchronized override fun onEvent(event: EngineEventFfi) { events.add(event) }
    @Synchronized fun count() = events.size
}

fun sh(vararg args: String) {
    val p = ProcessBuilder(*args).inheritIO().start()
    check(p.waitFor() == 0) { "command failed: ${args.joinToString(" ")}" }
}

fun main(argv: Array<String>) {
    val socket = argv[0]
    val fixtures = java.io.File("fixtures/agents").absolutePath

    check(engineVersion().count { it == '.' } == 2)
    check(cellAttrBits().bold.toInt() == 1)

    sh("tmux", "-L", socket, "-f", "/dev/null", "new-session", "-d", "-s", "agents",
       "-x", "90", "-y", "28", "$fixtures/fake-yn.sh")
    try {
        val engine = RemoteCoderEngine.connect(ConnConfigFfi.Local(socket))
        val listener = CollectingListener()
        engine.setListener(listener)

        val panes = engine.listPanes("agents")
        check(panes.size == 1)
        val pane = panes[0].id

        val grid = waitSnapshot(engine, pane, "Proceed? (y/n)")
        check(grid.cols.toInt() == 90 && grid.rows.toInt() == 28)
        check(grid.cursor != null)

        engine.attach(pane, 90u, 28u)
        engine.sendKeys(pane, "y")
        waitSnapshot(engine, pane, "proceeding")

        waitSnapshot(engine, pane, "Proceed? (y/n)")
        engine.pressButton(pane, "No")
        waitSnapshot(engine, pane, "step aborted.")

        check(listener.count() > 0) { "listener received no events" }
        println("[ffi-driver-kt] ALL FFI CHECKS PASSED (${listener.count()} events)")
    } finally {
        ProcessBuilder("tmux", "-L", socket, "kill-server").start().waitFor()
    }
}
