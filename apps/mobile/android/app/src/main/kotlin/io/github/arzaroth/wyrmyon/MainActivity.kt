package io.github.arzaroth.wyrmyon

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private lateinit var channel: MethodChannel
    private var picking: MethodChannel.Result? = null
    private var pendingIntent: Map<String, Any>? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Notifications.createChannels(this)
        if (savedInstanceState == null) {
            pendingIntent = describe(intent)
        }
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        channel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "wyrmyon/platform")
        channel.setMethodCallHandler(::handle)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        describe(intent)?.let { channel.invokeMethod("intent", it) }
    }

    private fun handle(call: MethodCall, result: MethodChannel.Result) {
        try {
            when (call.method) {
                "takeIntent" -> {
                    result.success(pendingIntent)
                    pendingIntent = null
                }
                "pickFiles" -> pickFiles(result)
                "openRead" -> result.success(openRead(Uri.parse(call.argument<String>("uri"))))
                "cacheDir" -> result.success(cacheDir.absolutePath)
                "createDownload" -> result.success(Downloads.create(this, call.argument<String>("name")!!))
                "finishDownload" -> {
                    Downloads.finish(this, Uri.parse(call.argument<String>("uri")), call.argument<Boolean>("kept")!!)
                    result.success(null)
                }
                "recentDownloads" -> result.success(Downloads.recent(this, call.argument<Int>("limit")!!))
                "openDownload" -> result.success(view(Uri.parse(call.argument<String>("uri"))))
                "shareDownload" -> result.success(share(Uri.parse(call.argument<String>("uri"))))
                "transferStarted" -> {
                    askForNotifications()
                    TransferService.start(this, call.argument<String>("title")!!)
                    result.success(null)
                }
                "transferProgress" -> {
                    TransferService.progress(
                        this,
                        call.argument<String>("title")!!,
                        call.argument<String>("text")!!,
                        call.argument<Number>("bytes")!!.toLong(),
                        call.argument<Number>("total")!!.toLong(),
                    )
                    result.success(null)
                }
                "transferEnded" -> {
                    TransferService.stop(this, call.argument<String>("title")!!, call.argument<String>("text"))
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        } catch (e: Exception) {
            result.error("platform", e.message ?: e.javaClass.simpleName, null)
        }
    }

    private fun pickFiles(result: MethodChannel.Result) {
        picking?.success(emptyList<Any>())
        picking = result
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        startActivityForResult(intent, PICK_FILES)
    }

    @Deprecated("FlutterActivity still delivers results here")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != PICK_FILES) return
        val result = picking ?: return
        picking = null
        val uris = mutableListOf<Uri>()
        if (resultCode == RESULT_OK && data != null) {
            val clip = data.clipData
            if (clip != null) {
                for (i in 0 until clip.itemCount) uris.add(clip.getItemAt(i).uri)
            } else {
                data.data?.let(uris::add)
            }
        }
        try {
            result.success(uris.map(::item))
        } catch (e: Exception) {
            result.error("platform", e.message ?: e.javaClass.simpleName, null)
        }
    }

    private fun describe(intent: Intent?): Map<String, Any>? {
        intent ?: return null
        return when (intent.action) {
            Intent.ACTION_VIEW -> intent.dataString?.let { mapOf("kind" to "link", "link" to it) }
            Intent.ACTION_SEND -> {
                val stream = parcelable(intent)
                when {
                    stream != null -> files(listOf(stream))
                    intent.getStringExtra(Intent.EXTRA_TEXT) != null ->
                        mapOf("kind" to "text", "text" to intent.getStringExtra(Intent.EXTRA_TEXT)!!)
                    else -> null
                }
            }
            Intent.ACTION_SEND_MULTIPLE -> parcelables(intent)?.let(::files)
            else -> null
        }
    }

    private fun files(uris: List<Uri>): Map<String, Any>? = try {
        mapOf("kind" to "files", "files" to uris.map(::item))
    } catch (e: Exception) {
        mapOf("kind" to "error", "error" to (e.message ?: e.javaClass.simpleName))
    }

    @Suppress("DEPRECATION")
    private fun parcelable(intent: Intent): Uri? =
        if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            intent.getParcelableExtra(Intent.EXTRA_STREAM)
        }

    @Suppress("DEPRECATION")
    private fun parcelables(intent: Intent): List<Uri>? =
        if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
        }

    private fun item(uri: Uri): Map<String, Any> {
        var name: String? = null
        var size = -1L
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val nameAt = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    val sizeAt = cursor.getColumnIndex(OpenableColumns.SIZE)
                    if (nameAt >= 0 && !cursor.isNull(nameAt)) name = cursor.getString(nameAt)
                    if (sizeAt >= 0 && !cursor.isNull(sizeAt)) size = cursor.getLong(sizeAt)
                }
            }
        if (size < 0) {
            size = contentResolver.openFileDescriptor(uri, "r")?.use { it.statSize } ?: -1L
        }
        return mapOf(
            "uri" to uri.toString(),
            "name" to (name ?: uri.lastPathSegment ?: "file"),
            "size" to size,
        )
    }

    private fun openRead(uri: Uri): Int =
        try {
            contentResolver.openFileDescriptor(uri, "r")?.detachFd() ?: -1
        } catch (e: Exception) {
            -1
        }

    private fun view(uri: Uri): Boolean {
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, contentResolver.getType(uri))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        return launch(intent)
    }

    private fun share(uri: Uri): Boolean {
        val intent = Intent(Intent.ACTION_SEND)
            .setType(contentResolver.getType(uri) ?: "application/octet-stream")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        return launch(Intent.createChooser(intent, null))
    }

    private fun launch(intent: Intent): Boolean =
        try {
            startActivity(intent)
            true
        } catch (e: ActivityNotFoundException) {
            false
        }

    private fun askForNotifications() {
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATIONS)
        }
    }

    private companion object {
        const val PICK_FILES = 1
        const val NOTIFICATIONS = 2
    }
}
