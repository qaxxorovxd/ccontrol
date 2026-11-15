package dev.ccontrol.core.net

import dev.ccontrol.core.protocol.HelloAck
import dev.ccontrol.core.protocol.Message
import dev.ccontrol.core.protocol.ProtocolException
import dev.ccontrol.core.security.DeviceIdentity
import dev.ccontrol.core.security.Sas
import dev.ccontrol.core.security.hexToBytes
import dev.ccontrol.core.security.toHex
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

sealed interface PairingUpdate {
    /** Both screens must show this code; the desktop owner approves on the computer. */
    data class AwaitingApproval(val sas: String, val timeoutSecs: Int) : PairingUpdate
    data class Accepted(val grantedScopes: List<String>, val ack: HelloAck) : PairingUpdate
    data class Rejected(val reason: String) : PairingUpdate
}

/** Commit/reveal SAS pairing, mirroring `desktop/agent/src/pairing.rs`. */
object PairingFlow {
    suspend fun run(
        session: AgentSession,
        identity: DeviceIdentity,
        requestedScopes: List<String>,
        onUpdate: (PairingUpdate) -> Unit,
    ): PairingUpdate = withContext(Dispatchers.IO) {
        val nc = Sas.randomNonce()
        session.sendBlocking(Message.PairCommit(Sas.commitment(nc), requestedScopes))
        val ns = when (val m = session.readBlocking(30_000)) {
            is Message.PairNonce -> m.nonce.hexToBytes().also { if (it.size != Sas.NONCE_LEN) throw ProtocolException("bad nonce") }
            is Message.Error -> return@withContext PairingUpdate.Rejected(m.error.message)
            else -> throw ProtocolException("unexpected ${m::class.simpleName}")
        }
        session.sendBlocking(Message.PairReveal(nc.toHex()))
        val code = Sas.compute(session.serverFingerprint, identity.fingerprint, nc, ns)
        while (true) {
            // Approval may take up to the agent's timeout (120 s) plus slack.
            when (val m = session.readBlocking(150_000)) {
                is Message.PairAwaitingApproval -> onUpdate(PairingUpdate.AwaitingApproval(code, m.timeoutSecs))
                is Message.PairResult -> {
                    if (!m.accepted) return@withContext PairingUpdate.Rejected(m.reason ?: "Rejected on the computer")
                    val ack = when (val next = session.readBlocking(10_000)) {
                        is Message.HelloAckMsg -> next.ack
                        else -> throw ProtocolException("expected hello_ack after pairing")
                    }
                    return@withContext PairingUpdate.Accepted(m.grantedScopes, ack)
                }
                is Message.Error -> return@withContext PairingUpdate.Rejected(m.error.message)
                else -> throw ProtocolException("unexpected ${m::class.simpleName}")
            }
        }
        @Suppress("UNREACHABLE_CODE")
        throw IllegalStateException()
    }
}
