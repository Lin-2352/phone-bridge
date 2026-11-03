package dev.phonebridge.app

/** Notification line while a PC sees this screen: "use" only when touch can really work (Relay control on). */
fun screenShareText(controlOn: Boolean, inputIgnored: Boolean): String = when {
    controlOn -> "Your PC can see and use this screen"
    inputIgnored -> "View only. A touch or key from your PC was ignored: turn on $APP_NAME control"
    else -> "Your PC can see this screen (view only)"
}

/** The consent question's body, true to what the PC will be able to do. */
fun screenConsentBody(controlOn: Boolean): String =
    if (controlOn) "Your screen appears in a window on the PC, which can also tap and type here. Stop any time from $APP_NAME or the notification."
    else "Your screen appears in a window on the PC, view only: touch and typing need $APP_NAME control (Accessibility). Stop any time from $APP_NAME or the notification."

/** Said to the PC window (once per session) when its input reaches nothing. */
fun controlOffNotice(): String = "Touch and typing are off. Turn on $APP_NAME control on the phone"
