package io.github.arzaroth.wyrmyon

import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap

object Downloads {
    private val folder = Environment.DIRECTORY_DOWNLOADS + "/wyrmyon/"
    private val collection: Uri = MediaStore.Downloads.EXTERNAL_CONTENT_URI

    fun create(context: Context, name: String): Map<String, Any> {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.RELATIVE_PATH, folder)
            put(MediaStore.Downloads.IS_PENDING, 1)
            mimeType(name)?.let { put(MediaStore.Downloads.MIME_TYPE, it) }
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(collection, values) ?: error("the download could not be created")
        val fd = try {
            resolver.openFileDescriptor(uri, "w")?.detachFd() ?: error("the download could not be opened")
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
        return mapOf("uri" to uri.toString(), "fd" to fd)
    }

    fun finish(context: Context, uri: Uri, kept: Boolean) {
        val resolver = context.contentResolver
        if (kept) {
            resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        } else {
            resolver.delete(uri, null, null)
        }
    }

    fun recent(context: Context, limit: Int): List<Map<String, Any>> {
        val projection = arrayOf(
            MediaStore.Downloads._ID,
            MediaStore.Downloads.DISPLAY_NAME,
            MediaStore.Downloads.SIZE,
            MediaStore.Downloads.DATE_MODIFIED,
        )
        val found = mutableListOf<Map<String, Any>>()
        context.contentResolver.query(
            collection,
            projection,
            "${MediaStore.Downloads.RELATIVE_PATH} = ?",
            arrayOf(folder),
            "${MediaStore.Downloads.DATE_MODIFIED} DESC",
        )?.use { cursor ->
            while (found.size < limit && cursor.moveToNext()) {
                found.add(
                    mapOf(
                        "uri" to ContentUris.withAppendedId(collection, cursor.getLong(0)).toString(),
                        "name" to (cursor.getString(1) ?: ""),
                        "size" to cursor.getLong(2),
                        "modified" to cursor.getLong(3) * 1000,
                    ),
                )
            }
        }
        return found
    }

    private fun mimeType(name: String): String? =
        name.substringAfterLast('.', "").lowercase().takeIf { it.isNotEmpty() }
            ?.let { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) }
}
