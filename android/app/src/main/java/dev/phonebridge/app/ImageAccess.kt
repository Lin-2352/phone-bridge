package dev.phonebridge.app

/** How much of the gallery Relay may see. */
enum class ImageAccess { FULL, PARTIAL, NONE }

/**
 * The permissions to ask for. From Android 14 the prompt can grant only the photos the user picks
 * (`READ_MEDIA_VISUAL_USER_SELECTED`); declaring it keeps that choice instead of a temporary grant.
 */
fun imagePermissions(sdk: Int): Array<String> = when {
    sdk >= 34 -> arrayOf("android.permission.READ_MEDIA_IMAGES", "android.permission.READ_MEDIA_VISUAL_USER_SELECTED")
    sdk >= 33 -> arrayOf("android.permission.READ_MEDIA_IMAGES")
    else -> arrayOf("android.permission.READ_EXTERNAL_STORAGE")
}

/** Only [ImageAccess.FULL] sees new screenshots; picked photos never include them. */
fun imageAccess(sdk: Int, full: Boolean, partial: Boolean): ImageAccess = when {
    full -> ImageAccess.FULL
    sdk >= 34 && partial -> ImageAccess.PARTIAL
    else -> ImageAccess.NONE
}

/** Said when the user picked some photos instead of allowing all. */
const val PARTIAL_PHOTOS_TEXT =
    "$APP_NAME can only see the photos you picked, so new screenshots won't be sent. Choose Allow all in Android's settings."
