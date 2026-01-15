// Old Architecture implementation for React Native < 0.82
package com.breeztech.breezsdkspark

import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.facebook.react.module.annotations.ReactModule
import com.facebook.react.turbomodule.core.interfaces.CallInvokerHolder

@ReactModule(name = BreezSdkSparkReactNativeModule.NAME)
class BreezSdkSparkReactNativeModule(reactContext: ReactApplicationContext) :
  ReactContextBaseJavaModule(reactContext) {

  override fun getName(): String {
    return NAME
  }

  // Same JNI calls as new arch - the native code is identical
  external fun nativeInstallRustCrate(runtimePointer: Long, callInvoker: CallInvokerHolder): Boolean
  external fun nativeCleanupRustCrate(runtimePointer: Long): Boolean

  @ReactMethod(isBlockingSynchronousMethod = true)
  fun installRustCrate(): Boolean {
    val context = this.reactApplicationContext
    return nativeInstallRustCrate(
      context.javaScriptContextHolder!!.get(),
      context.jsCallInvokerHolder!!
    )
  }

  @ReactMethod(isBlockingSynchronousMethod = true)
  fun cleanupRustCrate(): Boolean {
    return nativeCleanupRustCrate(
      this.reactApplicationContext.javaScriptContextHolder!!.get()
    )
  }

  companion object {
    const val NAME = "BreezSdkSparkReactNative"

    init {
      System.loadLibrary("breeztech-breez-sdk-spark-react-native")
    }
  }
}
