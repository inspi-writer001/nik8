package com.communik8.nik8native

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Stores the session keypair's 32-byte seed encrypted with an AES-GCM key that lives
 * in the Android Keystore and never leaves it. The seed is generated on first use.
 */
class SessionKeyStore(context: Context) {
  private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  /** Returns the existing seed, or generates and stores a new one. Caller should zero it after use. */
  fun loadOrCreateSeed(): ByteArray {
    val stored = prefs.getString(KEY_SEED, null)
    if (stored != null) {
      return decrypt(Base64.decode(stored, Base64.NO_WRAP))
    }

    val seed = ByteArray(SEED_LEN).also { SecureRandom().nextBytes(it) }
    prefs.edit().putString(KEY_SEED, Base64.encodeToString(encrypt(seed), Base64.NO_WRAP)).apply()
    return seed
  }

  private fun encrypt(plaintext: ByteArray): ByteArray {
    val cipher = Cipher.getInstance(TRANSFORMATION)
    cipher.init(Cipher.ENCRYPT_MODE, keystoreKey())
    return cipher.iv + cipher.doFinal(plaintext)
  }

  private fun decrypt(blob: ByteArray): ByteArray {
    val iv = blob.copyOfRange(0, IV_LEN)
    val cipher = Cipher.getInstance(TRANSFORMATION)
    cipher.init(Cipher.DECRYPT_MODE, keystoreKey(), GCMParameterSpec(TAG_BITS, iv))
    return cipher.doFinal(blob, IV_LEN, blob.size - IV_LEN)
  }

  private fun keystoreKey(): SecretKey {
    val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
    (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }

    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
    generator.init(
      KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build()
    )
    return generator.generateKey()
  }

  private companion object {
    const val ANDROID_KEYSTORE = "AndroidKeyStore"
    const val KEY_ALIAS = "communik8.session-seed"
    const val PREFS = "communik8.session"
    const val KEY_SEED = "seed"
    const val TRANSFORMATION = "AES/GCM/NoPadding"
    const val SEED_LEN = 32
    const val IV_LEN = 12
    const val TAG_BITS = 128
  }
}
