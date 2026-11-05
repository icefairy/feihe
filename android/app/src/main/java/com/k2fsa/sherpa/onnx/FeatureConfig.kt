// Copyright (c) 2023 Xiaomi Corporation
// Vendored from github.com/k2-fsa/sherpa-onnx (Apache-2.0).
// Package/class/field names must stay verbatim: libsherpa-onnx-jni.so
// resolves JNI symbols by these exact names. See ../../../../handoff.md.
package com.k2fsa.sherpa.onnx

data class FeatureConfig(
    var sampleRate: Int = 16000,
    var featureDim: Int = 80,
    var dither: Float = 0.0f
)

fun getFeatureConfig(sampleRate: Int, featureDim: Int): FeatureConfig {
    return FeatureConfig(sampleRate = sampleRate, featureDim = featureDim)
}
