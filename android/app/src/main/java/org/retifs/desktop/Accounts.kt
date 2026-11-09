package org.retifs.desktop

import android.accounts.AbstractAccountAuthenticator
import android.accounts.Account
import android.accounts.AccountAuthenticatorResponse
import android.accounts.AccountManager
import android.app.Service
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.IBinder
import android.provider.ContactsContract
import android.util.Log
import org.json.JSONArray

/**
 * retifs in the system's accounts, as a CardDAV app does it: the "retifs"
 * account, and an address book account for each library holding contacts
 * (ADR-0024) or made for them (ADR-0032), named after the library. A contact
 * made in one goes to its library. The system's Contacts takes one schema
 * per app, so every address book has the editable one; one whose library
 * this device may only read has its contacts marked read-only instead. No
 * passwords: retifs holds the keys.
 */
object Accounts {
    const val TYPE = "org.retifs"
    const val BOOK = "org.retifs.addressbook"
    /** Address books of an earlier version: removed. */
    private const val OLD_READ_ONLY_BOOK = "org.retifs.addressbook.ro"
    /** An address book's user data: "1" if its library may be written here. */
    const val WRITABLE = "writable"
    val ACCOUNT = Account("retifs", TYPE)

    /** Makes the "retifs" account (once). It has no contacts of its own; its calendars are the libraries'. */
    fun ensure(context: Context) {
        val am = AccountManager.get(context)
        if (am.getAccountsByType(TYPE).isEmpty()) am.addAccountExplicitly(ACCOUNT, null, null)
        ContentResolver.setIsSyncable(ACCOUNT, ContactsContract.AUTHORITY, 0)
        if (context.checkSelfPermission(android.Manifest.permission.WRITE_CALENDAR) == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            ContentResolver.setIsSyncable(ACCOUNT, android.provider.CalendarContract.AUTHORITY, 1)
            ContentResolver.setSyncAutomatically(ACCOUNT, android.provider.CalendarContract.AUTHORITY, true)
            ContentResolver.addPeriodicSync(ACCOUNT, android.provider.CalendarContract.AUTHORITY, Bundle.EMPTY, 3600)
        }
    }

    /** Syncs the calendars now. */
    fun syncCalendars() {
        val extras = Bundle().apply {
            putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
            putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
        }
        ContentResolver.requestSync(ACCOUNT, android.provider.CalendarContract.AUTHORITY, extras)
    }

    /** The address books brought up to date, then each synced (off the main thread). */
    fun syncContacts(context: Context) {
        val app = context.applicationContext
        Thread({
            // At boot or after an update retifsd is still starting: tried
            // again for a few minutes.
            for (attempt in 1..20) {
                try {
                    keepAddressBooks(app)
                    return@Thread
                } catch (e: Exception) {
                    if (attempt == 20 || e.message?.contains("no retifsd") != true) {
                        Log.w(ContactsSync.TAG, "address books: ${e.message}")
                        return@Thread
                    }
                    Thread.sleep(15_000)
                }
            }
        }, "retifs-address-books").start()
    }

    private fun request(account: Account) {
        val extras = Bundle().apply {
            putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
            putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
        }
        ContentResolver.requestSync(account, ContactsContract.AUTHORITY, extras)
    }

    /**
     * An address book account for each library holding contacts or made for
     * them; those of libraries gone removed, their contacts with them. Each
     * says whether its library may be written here, and is synced now.
     */
    fun keepAddressBooks(context: Context) {
        Core.ready(context)
        val libs = ContactsSync.parse(Core.contactLibraries()) as JSONArray
        val want = HashMap<String, Boolean>()
        for (i in 0 until libs.length()) {
            val l = libs.getJSONObject(i)
            want[l.getString("ns")] = l.getBoolean("writable")
        }
        val am = AccountManager.get(context)
        for (a in am.getAccountsByType(OLD_READ_ONLY_BOOK)) am.removeAccountExplicitly(a)
        for (a in am.getAccountsByType(BOOK)) {
            if (a.name !in want) {
                Log.i(ContactsSync.TAG, "address book ${a.name} removed")
                am.removeAccountExplicitly(a)
            }
        }
        for ((ns, writable) in want) {
            val a = Account(ns, BOOK)
            if (am.getAccountsByType(BOOK).none { it.name == ns }) {
                am.addAccountExplicitly(a, null, null)
                Log.i(ContactsSync.TAG, "address book $ns added")
            }
            am.setUserData(a, WRITABLE, if (writable) "1" else "0")
            ContentResolver.setIsSyncable(a, ContactsContract.AUTHORITY, 1)
            ContentResolver.setSyncAutomatically(a, ContactsContract.AUTHORITY, true)
            ContentResolver.addPeriodicSync(a, ContactsContract.AUTHORITY, Bundle.EMPTY, 3600)
            request(a)
        }
    }
}

/** An account's authenticator: nothing to authenticate, it's this phone's. */
class Authenticator(context: Context) : AbstractAccountAuthenticator(context) {
    override fun editProperties(r: AccountAuthenticatorResponse?, accountType: String?): Bundle? = null
    override fun addAccount(r: AccountAuthenticatorResponse?, accountType: String?, authTokenType: String?, features: Array<out String>?, options: Bundle?): Bundle? = null
    override fun confirmCredentials(r: AccountAuthenticatorResponse?, account: Account?, options: Bundle?): Bundle? = null
    override fun getAuthToken(r: AccountAuthenticatorResponse?, account: Account?, authTokenType: String?, options: Bundle?): Bundle? = null
    override fun getAuthTokenLabel(authTokenType: String?): String? = null
    override fun updateCredentials(r: AccountAuthenticatorResponse?, account: Account?, authTokenType: String?, options: Bundle?): Bundle? = null
    override fun hasFeatures(r: AccountAuthenticatorResponse?, account: Account?, features: Array<out String>?): Bundle? = null
}

/** The "retifs" account's authenticator service. */
open class AccountService : Service() {
    private val authenticator by lazy { Authenticator(this) }
    override fun onBind(intent: Intent?): IBinder = authenticator.iBinder
}

/** The address books' authenticator service. */
class AddressBookAccountService : AccountService()
