package de.mira.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin

enum class OrbMode { Idle, Listening, Thinking, Speaking, Error }

private val Cyan = Color(0xFF4FD1FF)
private val Amber = Color(0xFFFFAA46)

/** Der Mira-Ring – zugleich die Mikrofon-Taste. Gleicher Entwurf wie das PC-HUD. */
@Composable
fun MiraOrb(mode: OrbMode, level: Float, size: Dp, onClick: () -> Unit) {
    val currentMode by rememberUpdatedState(mode)
    val currentLevel by rememberUpdatedState(level)
    var t by remember { mutableFloatStateOf(0f) }
    var spin by remember { mutableFloatStateOf(0f) }
    var lvl by remember { mutableFloatStateOf(0f) }

    LaunchedEffect(Unit) {
        var last = 0L
        while (true) {
            withFrameNanos { now ->
                if (last != 0L) {
                    val dt = ((now - last) / 1e9f).coerceAtMost(0.1f)
                    t += dt
                    spin += dt * if (currentMode == OrbMode.Thinking) 2.4f else 0.35f
                    val target = when (currentMode) {
                        OrbMode.Listening -> currentLevel
                        OrbMode.Speaking -> 0.35f + 0.3f * abs(sin(t * 9f)) * abs(sin(t * 2.3f))
                        else -> 0f
                    }
                    lvl += (target - lvl) * min(1f, dt * 12f)
                }
                last = now
            }
        }
    }

    val label = when (mode) {
        OrbMode.Idle -> "ANTIPPEN ZUM SPRECHEN"
        OrbMode.Listening -> "HÖRE ZU"
        OrbMode.Thinking -> "VERARBEITE"
        OrbMode.Speaking -> "SPRECHE"
        OrbMode.Error -> "PROBLEM"
    }
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Canvas(
            Modifier
                .size(size)
                .clip(CircleShape)
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClick)
                .semantics { contentDescription = if (mode == OrbMode.Listening) "Zuhören beenden" else "Sprechen" },
        ) {
            val color = if (mode == OrbMode.Error) Amber else Cyan
            val dim = if (mode == OrbMode.Idle) 0.6f else 1f
            fun c(a: Float) = color.copy(alpha = (a * dim).coerceIn(0f, 1f))
            val s = this.size.minDimension / 2f / 172f
            val cx = center
            fun at(angle: Float, r: Float) = Offset(cx.x + cos(angle) * r * s, cx.y + sin(angle) * r * s)
            fun arc(r: Float, startRad: Float, sweepRad: Float, width: Float, a: Float) = drawArc(
                color = c(a),
                startAngle = Math.toDegrees(startRad.toDouble()).toFloat(),
                sweepAngle = Math.toDegrees(sweepRad.toDouble()).toFloat(),
                useCenter = false,
                topLeft = Offset(cx.x - r * s, cx.y - r * s),
                size = Size(2 * r * s, 2 * r * s),
                style = Stroke(width * s, cap = StrokeCap.Round),
            )
            val tau = (2 * PI).toFloat()

            for (i in 0 until 8) drawCircle(c(0.03f), radius = (64f + i * 13f + lvl * 22f) * s, center = cx)
            for (i in 0 until 72) {
                val ang = i / 72f * tau + spin * 0.25f
                val major = i % 6 == 0
                val len = if (major) 13f else 5f
                drawLine(c(if (major) 0.9f else 0.4f), at(ang, 168f - len), at(ang, 168f), strokeWidth = (if (major) 2f else 1.2f) * s)
            }
            drawCircle(c(0.25f), radius = 152f * s, center = cx, style = Stroke(1f * s))
            for (k in 0 until 3) arc(140f, -spin * 1.3f + k * tau / 3f, tau / 3f * 0.62f, 4f, 0.85f)
            for (k in 0 until 36) arc(121f, spin * 0.8f + k * tau / 36f, tau / 36f * 0.45f, 2f, 0.5f)
            for (k in 0 until 2) drawCircle(c(1f), radius = 3.5f * s, center = at(t * 1.7f + k * PI.toFloat(), 152f))
            for (i in 0 until 64) {
                val ang = i / 64f * tau - tau / 4f
                val wobble = 0.55f + 0.45f * sin(i * 2.3f + t * 9f)
                val amp = if (mode == OrbMode.Thinking) 0.15f + 0.12f * abs(sin(i * 0.5f - t * 7f)) else lvl * wobble
                drawLine(c(0.55f + 0.45f * amp), at(ang, 80f), at(ang, 84f + amp * 30f), strokeWidth = 2.4f * s)
            }
            val core = 46f + lvl * 10f
            drawCircle(Color(0xE6061622), radius = core * s, center = cx)
            drawCircle(c(1f), radius = core * s, center = cx, style = Stroke(2f * s))
            drawCircle(c(0.3f), radius = (20f + lvl * 14f) * s, center = cx)
            drawCircle(Color(0xFFDCFAFF).copy(alpha = dim), radius = (7f + lvl * 4f) * s, center = cx)
        }
        Text(
            label,
            color = if (mode == OrbMode.Error) Amber else Cyan,
            fontFamily = FontFamily.Monospace,
            fontSize = 12.sp,
            style = MaterialTheme.typography.labelMedium,
        )
    }
}
