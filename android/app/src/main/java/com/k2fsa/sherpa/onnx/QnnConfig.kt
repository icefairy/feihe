// Copyright (c) 2023 Xiaomi Corporation
// Vendored from github.com/k2-fsa/sherpa-onnx (Apache-2.0).
// Package/class/field names must stay verbatim: libsherpa-onnx-jni.so
// resolves JNI symbols by these exact names. See ../../../../handoff.md.
package com.k2fsa.sherpa.onnx

data class QnnConfig(
    var backendLib: String = "",
    var contextBinary: String = "",
    var systemLib: String = "",
)
