package dev.phonebridge.app

import org.junit.Assert.assertEquals
import org.junit.Test

class DecodeSizeTest {
    @Test
    fun ordinary_photos_decode_at_full_size() {
        assertEquals(1, decodeSampleSize(4000, 3000))
        assertEquals(1, decodeSampleSize(4096, 4096)) // exactly 16 MP
    }

    @Test
    fun big_pictures_are_downsampled_by_powers_of_two() {
        assertEquals(2, decodeSampleSize(4097, 4096))
        assertEquals(4, decodeSampleSize(12000, 9000)) // 108 MP -> 6.75 MP
    }

    @Test
    fun unknown_sizes_decode_as_is() {
        assertEquals(1, decodeSampleSize(0, 0))
        assertEquals(1, decodeSampleSize(-1, 500))
    }
}
