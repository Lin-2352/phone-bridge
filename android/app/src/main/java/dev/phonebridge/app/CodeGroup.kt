package dev.phonebridge.app

/** "628183" -> "628 183": digits in groups of three from the left. Anything that is not all digits is returned as typed (trimmed). */
fun groupCode(code: String): String {
    val s = code.trim()
    val digits = s.filterNot { it.isWhitespace() }
    if (digits.isEmpty() || !digits.all { it in '0'..'9' }) return s
    return digits.chunked(3).joinToString(" ")
}
