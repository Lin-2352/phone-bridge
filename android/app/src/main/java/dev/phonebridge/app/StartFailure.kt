package dev.phonebridge.app

import uniffi.pb_ffi.PbException

/** What a failed node start means for the next step (M3.7, B5). */
enum class StartFailure { RETRY, KEYSTORE_RESET }

/** The Keystore key no longer opens the sealed identity: it was reset (a new lock screen, a restore). */
private val KEY_GONE = listOf("AEADBadTagException", "sealed blob too short", "32-byte seed")

/**
 * Only an identity the Keystore can no longer open asks for a reset. A Keystore that is merely busy (for
 * example a `KeyStoreException` early after boot) keeps retrying: a reset there would throw pairings away.
 */
fun startFailure(e: Throwable): StartFailure =
    if (e is PbException.IdentityUnreadable && KEY_GONE.any { e.reason.contains(it) }) StartFailure.KEYSTORE_RESET else StartFailure.RETRY

/** Whether the recovery job should start Relay again: never while its identity is lost, or it would fail forever. */
fun shouldRecover(enabled: Boolean, running: Boolean, identityLost: Boolean): Boolean = enabled && !running && !identityLost
