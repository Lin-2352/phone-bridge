package dev.phonebridge.app

import android.media.MediaCodecInfo
import android.media.MediaCodecInfo.CodecCapabilities
import android.media.MediaCodecInfo.CodecProfileLevel
import android.media.MediaCodecInfo.EncoderCapabilities
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build

/** pb_video::Codec values. */
const val CODEC_H265_MAIN10 = 1
const val CODEC_H265 = 2
const val CODEC_H264 = 3

/**
 * What this phone's hardware video encoders can do for screen sharing:
 * which codecs (best first), VBR, a QP ceiling, size alignment.
 */
data class EncoderCaps(
    /** pb_video::Codec values, best first. */
    val codecs: List<Int>,
    val vbr: Boolean,
    val qpMax: Boolean,
    val alignW: Int,
    val alignH: Int,
    val encoderName: String,
) {
    companion object {
        @Volatile private var cached: EncoderCaps? = null

        fun get(): EncoderCaps = cached ?: probe().also { cached = it }

        /** Hardware encoders only; a surface input is required for screen capture. */
        fun probe(): EncoderCaps {
            val encoders = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter {
                it.isEncoder && (Build.VERSION.SDK_INT < 29 || it.isHardwareAccelerated)
            }
            fun find(mime: String): Pair<MediaCodecInfo, CodecCapabilities>? = encoders.firstNotNullOfOrNull { info ->
                if (info.supportedTypes.none { it.equals(mime, ignoreCase = true) }) return@firstNotNullOfOrNull null
                val caps = runCatching { info.getCapabilitiesForType(mime) }.getOrNull() ?: return@firstNotNullOfOrNull null
                if (CodecCapabilities.COLOR_FormatSurface !in caps.colorFormats) null else info to caps
            }
            val hevc = find(MediaFormat.MIMETYPE_VIDEO_HEVC)
            val avc = find(MediaFormat.MIMETYPE_VIDEO_AVC)
            val codecs = buildList {
                if (hevc != null) {
                    if (hevc.second.profileLevels.any { it.profile == CodecProfileLevel.HEVCProfileMain10 }) add(CODEC_H265_MAIN10)
                    add(CODEC_H265)
                }
                if (avc != null) add(CODEC_H264)
            }
            val best = hevc ?: avc
            val enc: EncoderCapabilities? = best?.second?.encoderCapabilities
            val video = best?.second?.videoCapabilities
            return EncoderCaps(
                codecs = codecs,
                vbr = enc?.isBitrateModeSupported(EncoderCapabilities.BITRATE_MODE_VBR) == true,
                qpMax = Build.VERSION.SDK_INT >= 31,
                alignW = maxOf(16, video?.widthAlignment ?: 16),
                alignH = maxOf(16, video?.heightAlignment ?: 16),
                encoderName = best?.first?.name ?: "none",
            )
        }
    }

    /** The encoder for one of our codec values, by MIME type. */
    fun mime(codec: Int): String =
        if (codec == CODEC_H264) MediaFormat.MIMETYPE_VIDEO_AVC else MediaFormat.MIMETYPE_VIDEO_HEVC
}
