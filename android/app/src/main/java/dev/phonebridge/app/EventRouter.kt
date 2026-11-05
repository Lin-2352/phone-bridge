package dev.phonebridge.app

import java.util.concurrent.Executor
import uniffi.pb_ffi.PbEvent

/**
 * Received files are copied into MediaStore or the chosen folder on a thread of their own, in
 * order: a multi-GB copy must not hold up the PC's control, screen or audio question (B1).
 */
class EventRouter(private val events: Executor, private val saves: Executor) {
    fun route(e: PbEvent, handle: (PbEvent) -> Unit) {
        (if (e is PbEvent.FileReceived) saves else events).execute { handle(e) }
    }
}
