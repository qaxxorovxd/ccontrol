package dev.ccontrol.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import dev.ccontrol.app.ui.screens.AddDeviceScreen
import dev.ccontrol.app.ui.screens.AppsScreen
import dev.ccontrol.app.ui.screens.ProcessesScreen
import dev.ccontrol.app.ui.screens.DashboardScreen
import dev.ccontrol.app.ui.screens.DeviceScreen
import dev.ccontrol.app.ui.screens.PairingScreen
import dev.ccontrol.app.ui.screens.PairingTarget
import dev.ccontrol.app.ui.theme.CControlTheme
import dev.ccontrol.app.ui.theme.Palette
import java.net.URLDecoder
import java.net.URLEncoder

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val app = application as CControlApp
        setContent {
            CControlTheme {
                Box(Modifier.fillMaxSize().background(Palette.Background)) {
                    val nav = rememberNavController()
                    NavHost(nav, startDestination = "dashboard") {
                        composable("dashboard") {
                            DashboardScreen(
                                app = app,
                                onOpen = { nav.navigate("device/$it") },
                                onAdd = { nav.navigate("add") },
                            )
                        }
                        composable("add") {
                            AddDeviceScreen(
                                app = app,
                                onBack = { nav.popBackStack() },
                                onPair = { t -> nav.navigate("pair/${enc(t.host)}/${t.port}/${enc(t.fingerprint ?: "-")}/${enc(t.name ?: "-")}") },
                            )
                        }
                        composable(
                            "pair/{host}/{port}/{fp}/{name}",
                            arguments = listOf(navArgument("port") { type = NavType.IntType }),
                        ) { e ->
                            val a = e.arguments!!
                            val target = PairingTarget(
                                host = dec(a.getString("host")!!),
                                port = a.getInt("port"),
                                fingerprint = dec(a.getString("fp")!!).takeIf { it != "-" },
                                name = dec(a.getString("name")!!).takeIf { it != "-" },
                            )
                            PairingScreen(
                                app = app,
                                target = target,
                                onDone = { id ->
                                    nav.popBackStack("dashboard", inclusive = false)
                                    if (id != null) nav.navigate("device/$id")
                                },
                                onCancel = { nav.popBackStack() },
                            )
                        }
                        composable("device/{id}") { e ->
                            val id = e.arguments!!.getString("id")!!
                            DeviceScreen(
                                app = app, deviceId = id, onBack = { nav.popBackStack() },
                                onOpenProcesses = { nav.navigate("device/$id/processes") },
                                onOpenApps = { nav.navigate("device/$id/apps") },
                            )
                        }
                        composable("device/{id}/processes") { e ->
                            ProcessesScreen(app = app, deviceId = e.arguments!!.getString("id")!!, onBack = { nav.popBackStack() })
                        }
                        composable("device/{id}/apps") { e ->
                            AppsScreen(app = app, deviceId = e.arguments!!.getString("id")!!, onBack = { nav.popBackStack() })
                        }
                    }
                }
            }
        }
    }

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8")
    private fun dec(s: String) = URLDecoder.decode(s, "UTF-8")
}
