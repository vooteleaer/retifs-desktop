package org.retifs.desktop

import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import android.webkit.MimeTypeMap
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileNotFoundException

/**
 * The libraries in Android's own file places: the Files app's drawer and
 * every app's file picker (Storage Access Framework), as Cloud Files does on
 * Windows and FUSE on Linux. One root, "retifs", with a folder per library;
 * in each, the same tree as the other mounts (a record's path, else Unfiled).
 *
 * A file opened for reading is fetched whole into the cache first (from
 * other nodes if it isn't on the phone); one opened for writing is written
 * to a cache file and stored when the app closes it, as one new version.
 * retifsd does the rest: keys, sync, what's kept. Nothing is held here.
 *
 * Document IDs: "root", then "<library>:<path>" ("photos:/", "photos:/a.jpg").
 */
class LibrariesProvider : DocumentsProvider() {
    private lateinit var writes: Handler

    override fun onCreate(): Boolean {
        Core.ready(context!!)
        writes = Handler(HandlerThread("retifs-documents").apply { start() }.looper)
        return true
    }

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val c = MatrixCursor(projection ?: ROOT_COLUMNS)
        c.newRow().apply {
            add(Root.COLUMN_ROOT_ID, ROOT)
            add(Root.COLUMN_DOCUMENT_ID, ROOT)
            add(Root.COLUMN_TITLE, "retifs")
            add(Root.COLUMN_SUMMARY, "Your libraries, encrypted, on every device")
            add(Root.COLUMN_ICON, R.mipmap.ic_launcher)
            add(Root.COLUMN_FLAGS, Root.FLAG_SUPPORTS_CREATE or Root.FLAG_SUPPORTS_IS_CHILD)
        }
        return c
    }

    override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor {
        val c = MatrixCursor(projection ?: DOCUMENT_COLUMNS)
        if (documentId == ROOT) {
            row(c, ROOT, "retifs", true, 0, null, writable = false)
            return c
        }
        val (ns, path) = split(documentId)
        if (path == "/") {
            row(c, documentId, ns, true, 0, null, writable = true)
            return c
        }
        val answer = Core.docStat(ns, path)
        if (answer == "null") throw FileNotFoundException("$path isn't in $ns")
        val e = parse(answer) as JSONObject
        row(c, documentId, e.getString("name"), e.getBoolean("dir"), e.optLong("size"), e.optLong("mtime").takeIf { !e.isNull("mtime") }, writable = true)
        return c
    }

    override fun queryChildDocuments(parentDocumentId: String, projection: Array<out String>?, sortOrder: String?): Cursor {
        val c = MatrixCursor(projection ?: DOCUMENT_COLUMNS)
        if (parentDocumentId == ROOT) {
            val names = parse(Core.docLibraries()) as JSONArray
            for (i in 0 until names.length()) {
                val ns = names.getString(i)
                row(c, id(ns, "/"), ns, true, 0, null, writable = true)
            }
        } else {
            val (ns, dir) = split(parentDocumentId)
            val entries = parse(Core.docList(ns, dir)) as JSONArray
            for (i in 0 until entries.length()) {
                val e = entries.getJSONObject(i)
                row(c, id(ns, e.getString("path")), e.getString("name"), e.getBoolean("dir"), e.optLong("size"), e.optLong("mtime").takeIf { !e.isNull("mtime") }, writable = true)
            }
        }
        c.setNotificationUri(context!!.contentResolver, DocumentsContract.buildChildDocumentsUri(AUTHORITY, parentDocumentId))
        return c
    }

    override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean {
        if (parentDocumentId == ROOT) return documentId != ROOT
        val (pns, pdir) = split(parentDocumentId)
        if (documentId == ROOT) return false
        val (ns, path) = split(documentId)
        return ns == pns && path != pdir && (pdir == "/" || path.startsWith("$pdir/"))
    }

    override fun openDocument(documentId: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
        val (ns, path) = split(documentId)
        val file = File(cache(), "${System.nanoTime()}-${path.substringAfterLast('/')}")
        val pfdMode = ParcelFileDescriptor.parseMode(mode)
        val writing = mode.contains('w') || mode.contains('a')
        // Read, or written on top of what's there ("rw", "wa"): the content first.
        if (!writing || mode.contains('r') || mode.contains('a')) {
            val error = Core.docRead(ns, path, file.absolutePath)
            if (error.isNotEmpty()) {
                file.delete()
                throw FileNotFoundException(error)
            }
        }
        if (!writing) {
            // Gone from the cache once the reader closes it.
            return ParcelFileDescriptor.open(file, pfdMode, writes) { file.delete() }
        }
        return ParcelFileDescriptor.open(file, pfdMode, writes) { e ->
            // The app closed it: stored as the file's next version.
            if (e == null) {
                val error = Core.docWrite(ns, path, file.absolutePath)
                if (error.isNotEmpty()) android.util.Log.w(TAG, "$ns:$path not stored: $error")
                changed(ns, path)
            }
            file.delete()
        }
    }

    override fun createDocument(parentDocumentId: String, mimeType: String, displayName: String): String {
        if (parentDocumentId == ROOT) throw FileNotFoundException("a library is made in retifs, not here")
        val (ns, dir) = split(parentDocumentId)
        val path = free(ns, dir, displayName)
        if (mimeType == Document.MIME_TYPE_DIR) {
            check(Core.docMkdir(ns, path))
        } else {
            // An empty file now; its content comes when the app writes it.
            val empty = File(cache(), "${System.nanoTime()}-new").apply { writeBytes(ByteArray(0)) }
            try {
                check(Core.docWrite(ns, path, empty.absolutePath))
            } finally {
                empty.delete()
            }
        }
        changed(ns, path)
        return id(ns, path)
    }

    override fun deleteDocument(documentId: String) {
        val (ns, path) = split(documentId)
        check(Core.docRemove(ns, path))
        changed(ns, path)
    }

    override fun renameDocument(documentId: String, displayName: String): String {
        val (ns, path) = split(documentId)
        val to = join(parent(path), displayName)
        check(Core.docRename(ns, path, to))
        changed(ns, path)
        return id(ns, to)
    }

    override fun moveDocument(sourceDocumentId: String, sourceParentDocumentId: String, targetParentDocumentId: String): String {
        val (ns, path) = split(sourceDocumentId)
        if (targetParentDocumentId == ROOT) throw FileNotFoundException("only into a library's folder")
        val (tns, tdir) = split(targetParentDocumentId)
        // Records belong to one library: copied across (the system does), not moved.
        if (tns != ns) throw FileNotFoundException("to another library, copy it instead")
        val to = join(tdir, path.substringAfterLast('/'))
        check(Core.docRename(ns, path, to))
        changed(ns, path)
        changed(ns, to)
        return id(ns, to)
    }

    private fun row(c: MatrixCursor, id: String, name: String, dir: Boolean, size: Long, mtime: Long?, writable: Boolean) {
        var flags = 0
        if (writable && id != ROOT) {
            flags = Document.FLAG_SUPPORTS_DELETE or Document.FLAG_SUPPORTS_RENAME or Document.FLAG_SUPPORTS_MOVE
            if (dir) flags = flags or Document.FLAG_DIR_SUPPORTS_CREATE else flags = flags or Document.FLAG_SUPPORTS_WRITE
            // A library's own folder: left in retifs, not deleted or renamed here.
            if (split(id).second == "/") flags = Document.FLAG_DIR_SUPPORTS_CREATE
        }
        c.newRow().apply {
            add(Document.COLUMN_DOCUMENT_ID, id)
            add(Document.COLUMN_DISPLAY_NAME, name)
            add(Document.COLUMN_MIME_TYPE, if (dir) Document.MIME_TYPE_DIR else mime(name))
            add(Document.COLUMN_SIZE, if (dir) null else size)
            add(Document.COLUMN_LAST_MODIFIED, mtime)
            add(Document.COLUMN_FLAGS, flags)
        }
    }

    /** A name not taken in `dir`: "a.jpg", else "a (2).jpg", … */
    private fun free(ns: String, dir: String, name: String): String {
        val stem = name.substringBeforeLast('.', name)
        val ext = if (name.contains('.')) "." + name.substringAfterLast('.') else ""
        var candidate = join(dir, name)
        for (n in 2..1000) {
            val answer = Core.docStat(ns, candidate)
            if (answer == "null") return candidate
            parse(answer) // an error is thrown, not taken for a name in use
            candidate = join(dir, "$stem ($n)$ext")
        }
        throw FileNotFoundException("no free name for $name in $dir")
    }

    private fun changed(ns: String, path: String) {
        context!!.contentResolver.notifyChange(DocumentsContract.buildChildDocumentsUri(AUTHORITY, id(ns, parent(path))), null)
    }

    private fun cache(): File = File(context!!.cacheDir, "documents").apply { mkdirs() }

    companion object {
        const val AUTHORITY = "org.retifs.desktop.documents"
        private const val ROOT = "root"
        private const val TAG = "retifs-documents"
        private val ROOT_COLUMNS = arrayOf(Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_SUMMARY, Root.COLUMN_ICON, Root.COLUMN_FLAGS)
        private val DOCUMENT_COLUMNS = arrayOf(
            Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE,
            Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS,
        )

        fun id(ns: String, path: String) = "$ns:$path"

        fun split(id: String): Pair<String, String> {
            val i = id.indexOf(':')
            if (i <= 0) throw FileNotFoundException("not a retifs document: $id")
            return id.substring(0, i) to id.substring(i + 1).ifEmpty { "/" }
        }

        fun parent(path: String): String = path.substringBeforeLast('/', "").ifEmpty { "/" }

        fun join(dir: String, name: String): String = if (dir == "/") "/$name" else "$dir/$name"

        fun mime(name: String): String =
            MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"

        /** A core answer as JSON; `{"error"}` thrown as a missing file, which the system shows. */
        fun parse(answer: String): Any {
            val v: Any = if (answer.trimStart().startsWith("[")) JSONArray(answer) else JSONObject(answer)
            if (v is JSONObject && v.has("error")) throw FileNotFoundException(v.getString("error"))
            return v
        }

        /** A core error ("" when fine) thrown as a missing file. */
        fun check(error: String) {
            if (error.isNotEmpty()) throw FileNotFoundException(error)
        }
    }
}
