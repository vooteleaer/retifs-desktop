package org.retifs.desktop

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Starts retifs when the phone starts, and again after an update (which
 * stops it), as the desktop utility starts at login.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> RetifsService.start(context)
        }
    }
}
