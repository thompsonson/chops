package dev.chops.app

import android.os.Bundle
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebView

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setupWebViewPermissions()
    }

    private fun setupWebViewPermissions() {
        val rootView = findViewById<android.view.View>(android.R.id.content) ?: return
        rootView.viewTreeObserver.addOnGlobalLayoutListener(object :
            android.view.ViewTreeObserver.OnGlobalLayoutListener {
            override fun onGlobalLayout() {
                val webView = findWebView(rootView) ?: return
                rootView.viewTreeObserver.removeOnGlobalLayoutListener(this)
                webView.webChromeClient = object : WebChromeClient() {
                    override fun onPermissionRequest(request: PermissionRequest) {
                        runOnUiThread { request.grant(request.resources) }
                    }
                }
            }
        })
    }

    private fun findWebView(view: android.view.View?): WebView? {
        if (view is WebView) return view
        if (view is android.view.ViewGroup) {
            for (i in 0 until view.childCount) {
                val found = findWebView(view.getChildAt(i))
                if (found != null) return found
            }
        }
        return null
    }
}
