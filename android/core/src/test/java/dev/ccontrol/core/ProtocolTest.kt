package dev.ccontrol.core

import dev.ccontrol.core.net.PairingUri
import dev.ccontrol.core.protocol.Action
import dev.ccontrol.core.protocol.ActionResult
import dev.ccontrol.core.protocol.Framing
import dev.ccontrol.core.protocol.MediaKey
import dev.ccontrol.core.protocol.Message
import dev.ccontrol.core.protocol.ProtocolException
import dev.ccontrol.core.protocol.Wire
import dev.ccontrol.core.security.Sas
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

/** JSON literals below are copied from `crates/protocol/src/messages.rs` tests. */
class ProtocolTest {
    @Test fun requestShapeMatchesRust() {
        assertEquals(
            """{"type":"request","id":3,"action":"volume.set","level":0.25}""",
            Wire.encode(Message.Request(3, Action.VolumeSet(0.25f))),
        )
        assertEquals(
            """{"type":"request","id":1,"action":"device.info"}""",
            Wire.encode(Message.Request(1, Action.DeviceInfo)),
        )
        assertEquals(
            """{"type":"request","id":2,"action":"media.key","key":"play_pause"}""",
            Wire.encode(Message.Request(2, Action.Media(MediaKey.PlayPause))),
        )
    }

    @Test fun decodesRustResponses() {
        val ok = Wire.decode("""{"type":"response","id":9,"result":{"kind":"volume","level":0.5,"muted":false}}""") as Message.Response
        assertEquals(9L, ok.id)
        assertEquals(0.5f, (ok.result as ActionResult.Volume).state.level)
        val err = Wire.decode("""{"type":"response","id":9,"error":{"code":"scope_denied","message":"x"}}""") as Message.Response
        assertNull(err.result)
        assertEquals("scope_denied", err.error!!.code)
        val ev = Wire.decode("""{"type":"event","event":"revoked","reason":"owner"}""") as Message.Event
        assertEquals("revoked", ev.event)
    }

    @Test fun decodesHelloAckWithManifest() {
        val m = Wire.decode(
            """{"type":"hello_ack","protocol_version":1,"device_id":"ab","device_name":"PC","paired":true,
               "granted_scopes":["device.read","media.control"],
               "manifest":{"device_id":"ab","device_name":"PC","os_family":"windows","os_distribution":null,
               "os_version":"Windows 10 Pro","os_build":"19045","kernel_version":"19045","architecture":"x86_64",
               "session_type":"windows","agent_version":"0.1.0","protocol_version":1,
               "capabilities":{"audio.master_volume":{"available":true,"scope":"media.control"},
               "audio.route":{"available":false,"reason":"Not implemented","scope":"audio.route"}},"future_field":1}}""",
        ) as Message.HelloAckMsg
        assertTrue(m.ack.paired)
        assertEquals("windows", m.ack.manifest!!.osFamily)
        assertEquals(false, m.ack.manifest.capabilities["audio.route"]!!.available)
    }

    @Test fun phase2ShapesMatchRust() {
        assertEquals(
            """{"type":"request","id":4,"action":"process.terminate","pid":42,"name":"notepad.exe","force":false}""",
            Wire.encode(Message.Request(4, Action.ProcessTerminate(42, "notepad.exe", false))),
        )
        assertEquals(
            """{"type":"request","id":5,"action":"app.close","window_id":7,"pid":42}""",
            Wire.encode(Message.Request(5, Action.AppClose(7, 42))),
        )
        assertEquals("""{"type":"request","id":6,"action":"pairing.unpair"}""", Wire.encode(Message.Request(6, Action.Unpair)))
        val t = Wire.decodeResult(Wire.json.parseToJsonElement("""{"kind":"terminated","pid":42,"method":"graceful","exited":true}""") as kotlinx.serialization.json.JsonObject)
        assertEquals(true, (t as ActionResult.Terminated).outcome.exited)
        val m = Wire.decodeResult(Wire.json.parseToJsonElement("""{"kind":"media_status","active":false,"status":"stopped","can_play_pause":false,"can_next":false,"can_previous":false}""") as kotlinx.serialization.json.JsonObject)
        assertEquals(false, (m as ActionResult.Media).status.active)
        val tel = Wire.decodeResult(Wire.json.parseToJsonElement("""{"kind":"telemetry","cpu_percent":1.0,"cpu_count":8,"memory_used_bytes":1,"memory_total_bytes":2,"uptime_secs":3,"disks":[{"mount":"C:\\","name":"","total_bytes":10,"available_bytes":5,"removable":false}]}""") as kotlinx.serialization.json.JsonObject)
        assertEquals("C:\\", (tel as ActionResult.TelemetryResult).telemetry.disks.single().mount)
    }

    @Test fun audioRouteShapes() {
        assertEquals(
            """{"type":"request","id":7,"action":"route.audio.start","route_id":"r1","source":{"kind":"session","pid":42,"app_name":"Spotify"},"dest_host":"10.0.0.2","dest_port":5000,"key":"k","lease_secs":30,"silence_local":true}""",
            Wire.encode(Message.Request(7, Action.RouteStart("r1", dev.ccontrol.core.protocol.RouteSource.Session(42, "Spotify"), "10.0.0.2", 5000, "k", 30, true))),
        )
        assertEquals(
            """{"type":"request","id":8,"action":"audio.sink.prepare","route_id":"r1","key":"k","lease_secs":30}""",
            Wire.encode(Message.Request(8, Action.SinkPrepare("r1", "k", null, 30))),
        )
        val r = Wire.decodeResult(Wire.json.parseToJsonElement("""{"kind":"route","route_id":"r1","state":"active","source":{"kind":"test_tone"},"capture":"test_tone","dest":"h:1","owner":"o","silence_local_requested":false,"local_silenced":false,"lease_remaining_ms":1000,"stats":{"packets":1,"lost":0,"late":0,"duplicates":0,"concealed_frames":0,"frames_played":1,"underruns":0,"jitter_ms":0.0,"buffer_ms":40.0,"bitrate_kbps":160.0,"peak":0.1}}""") as kotlinx.serialization.json.JsonObject)
        assertEquals("active", (r as ActionResult.Route).route.state)
    }

    @Test(expected = IllegalArgumentException::class) fun rejectsOutOfRangeVolume() {
        Action.VolumeSet(1.5f)
    }

    @Test(expected = ProtocolException::class) fun rejectsUnknownType() {
        Wire.decode("""{"type":"shell","cmd":"x"}""")
    }

    @Test fun framingRoundTrip() {
        val buf = ByteArrayOutputStream()
        Framing.write(buf, Message.Ping(7))
        val bytes = buf.toByteArray()
        assertEquals(0, bytes[0].toInt()); assertEquals(bytes.size - 4, bytes[3].toInt())
        // The agent echoes pongs; decode a pong frame built the same way.
        val pong = """{"type":"pong","seq":7}""".toByteArray()
        val frame = byteArrayOf(0, 0, 0, pong.size.toByte()) + pong
        assertEquals(Message.Pong(7), Framing.read(ByteArrayInputStream(frame)))
    }

    @Test(expected = ProtocolException::class) fun framingRejectsOversized() {
        Framing.read(ByteArrayInputStream(byteArrayOf(0x7f, 0, 0, 0)))
    }

    /** Same golden vector as `crates/security/src/sas.rs`. */
    @Test fun sasGoldenVector() {
        val sas = Sas.compute("1".repeat(64), "2".repeat(64), ByteArray(32) { 0xAB.toByte() }, ByteArray(32) { 0xCD.toByte() })
        assertEquals("285326", sas)
        assertEquals(
            "904fd8d0fbe2bc606afa210ab9b1dcf33974f2e7551da819d9a91318a237132f",
            Sas.commitment(ByteArray(32) { 0xAB.toByte() }),
        )
        assertNotEquals(sas, Sas.compute("1".repeat(64), "3".repeat(64), ByteArray(32) { 0xAB.toByte() }, ByteArray(32) { 0xCD.toByte() }))
    }

    @Test fun parsesPairingUri() {
        val fp = "fe183c41cb611dedfce519c0ff524e45ab3fa8f41c1a6a7af6c0a19e22967894"
        val u = PairingUri.parse("ccontrol://pair?h=192.168.1.14&p=47800&fp=$fp&n=DESKTOP-LMKPLCK")!!
        assertEquals("192.168.1.14", u.host); assertEquals(47800, u.port); assertEquals(fp, u.fingerprint)
        assertNull(PairingUri.parse("ccontrol://pair?h=1.2.3.4&p=47800&fp=short"))
        assertNull(PairingUri.parse("https://evil.example/pair?h=1.2.3.4&p=1&fp=$fp"))
        assertNull(PairingUri.parse("ccontrol://pair?h=a;rm -rf&p=1&fp=$fp"))
    }
}
