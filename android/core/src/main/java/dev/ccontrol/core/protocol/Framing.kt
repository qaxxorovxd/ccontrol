package dev.ccontrol.core.protocol

import java.io.DataInputStream
import java.io.EOFException
import java.io.InputStream
import java.io.OutputStream

/** 4-byte big-endian length prefix + UTF-8 JSON, identical to `crates/protocol/src/frame.rs`. */
object Framing {
    fun write(out: OutputStream, m: Message) {
        val body = Wire.encode(m).toByteArray(Charsets.UTF_8)
        if (body.size > ProtocolInfo.MAX_FRAME) throw ProtocolException("frame too large")
        val header = byteArrayOf(
            (body.size ushr 24).toByte(), (body.size ushr 16).toByte(),
            (body.size ushr 8).toByte(), body.size.toByte(),
        )
        synchronized(out) {
            out.write(header + body)
            out.flush()
        }
    }

    /** Blocking read of one frame. Throws [EOFException] when the peer closed the stream. */
    fun read(input: InputStream): Message {
        val din = if (input is DataInputStream) input else DataInputStream(input)
        val len = din.readInt()
        if (len <= 0 || len > ProtocolInfo.MAX_FRAME) throw ProtocolException("invalid frame length $len")
        val buf = ByteArray(len)
        din.readFully(buf)
        return Wire.decode(String(buf, Charsets.UTF_8))
    }
}
