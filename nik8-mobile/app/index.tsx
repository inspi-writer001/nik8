import { Pressable, StyleSheet, Text, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'
import { useRouter } from 'expo-router'
import { useMobileWallet } from '@wallet-ui/react-native-kit'
import React, { useState } from 'react'

export default function LandingScreen() {
  const router = useRouter()
  const { account, connect } = useMobileWallet()
  const [activating, setActivating] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function activate() {
    setError(null)
    setActivating(true)
    try {
      if (!account) {
        await connect()
      }
      router.replace('/talk')
    } catch (e) {
      setError(`Could not connect wallet: ${e}`)
    } finally {
      setActivating(false)
    }
  }

  return (
    <SafeAreaView style={styles.screen}>
      <View style={styles.brand}>
        <Text style={styles.brandText}>Communik8 protocol</Text>
      </View>

      <View style={styles.actions}>
        <Pressable
          accessibilityRole="button"
          disabled={activating}
          onPress={activate}
          style={({ pressed }) => [styles.button, (pressed || activating) && styles.buttonPressed]}
        >
          <Text style={[styles.buttonText, activating && styles.buttonTextPressed]}>
            {activating ? 'Activ8ing…' : 'Activ8 comms'}
          </Text>
        </Pressable>
        <Text style={styles.terms}>
          By activating comms you accept the <Text style={styles.termsLink}>terms</Text> to use this application.
        </Text>
        {error ? <Text style={styles.error}>{error}</Text> : null}
      </View>
    </SafeAreaView>
  )
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    backgroundColor: '#ffffff',
    paddingHorizontal: 32,
  },
  brand: {
    alignSelf: 'center',
    backgroundColor: '#000000',
    marginTop: 64,
    paddingHorizontal: 20,
    paddingVertical: 14,
  },
  brandText: {
    color: '#ffffff',
    fontSize: 22,
    fontStyle: 'italic',
    fontWeight: '300',
    letterSpacing: 0.5,
  },
  actions: {
    flex: 1,
    gap: 12,
    justifyContent: 'center',
    marginTop: 120,
  },
  button: {
    alignItems: 'center',
    borderColor: '#000000',
    borderWidth: 2,
    paddingVertical: 16,
  },
  buttonPressed: {
    backgroundColor: '#000000',
  },
  buttonText: {
    color: '#000000',
    fontSize: 20,
  },
  buttonTextPressed: {
    color: '#ffffff',
  },
  terms: {
    color: '#444444',
    fontSize: 13,
  },
  termsLink: {
    textDecorationLine: 'underline',
  },
  error: {
    color: '#b00020',
    fontSize: 13,
  },
})
