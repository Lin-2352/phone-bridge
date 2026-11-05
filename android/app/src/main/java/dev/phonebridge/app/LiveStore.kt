package dev.phonebridge.app

/** One button of a live notification: its intent and, for a reply button, its inputs. */
data class LiveAction<P, R>(val intent: P?, val inputs: R?)

/**
 * What Relay keeps of a live notification so a PC can act on it later: its category (calls), its
 * title (for "Tap to open"), its tap intent and its buttons. Never its pictures or extras.
 */
data class LiveNote<P, R>(val category: String?, val title: String?, val content: P?, val actions: List<LiveAction<P, R>>)

/** Key -> value, at most [max] entries; the least recently used goes first. Thread-safe. */
class LiveStore<V>(private val max: Int = 500) {
    private val map = object : LinkedHashMap<String, V>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, V>?): Boolean = size > max
    }

    /** True when an entry for [key] was replaced. */
    @Synchronized
    fun put(key: String, value: V): Boolean = map.put(key, value) != null

    @Synchronized
    fun get(key: String): V? = map[key]

    @Synchronized
    fun remove(key: String): V? = map.remove(key)

    @Synchronized
    fun clear() = map.clear()

    @Synchronized
    fun any(predicate: (V) -> Boolean): Boolean = map.values.any(predicate)
}
