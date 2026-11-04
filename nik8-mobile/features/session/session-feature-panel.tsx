import { ActivityIndicator, Pressable, StyleSheet, Text, View } from 'react-native'
import React from 'react'
import { ellipsify } from '@/utils/ellipsify'
import {
  LAMPORTS_PER_SOL,
  SESSION_FUNDING_LAMPORTS,
  SESSION_MIN_LAMPORTS,
  useSession,
} from '@/features/session/use-session'

function sol(lamports: number | bigint) {
  return (Number(lamports) / LAMPORTS_PER_SOL).toFixed(4)
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

/** Session key setup: fund (wallet popup) → register (no popup); top up when low. */
export function SessionFeaturePanel() {
  const { pubkey, status, fund, register } = useSession()

  if (pubkey.isPending || (pubkey.data && status.isPending)) {
    return (
      <View style={styles.panel}>
        <ActivityIndicator color="#000000" />
        <Text style={styles.detail}>Preparing session key…</Text>
      </View>
    )
  }

  if (pubkey.isError || status.isError) {
    return (
      <View style={styles.panel}>
        <Text style={styles.error}>{errorText(pubkey.error ?? status.error)}</Text>
        <ActionButton
          title="Retry"
          onPress={() => (pubkey.isError ? pubkey.refetch() : status.refetch())}
        />
      </View>
    )
  }

  const session = status.data
  if (!session) return null

  const lastError = fund.error ?? register.error
  const busy = fund.isPending || register.isPending

  let action: React.ReactNode = null
  if (session.balanceLamports < SESSION_MIN_LAMPORTS) {
    action = (
      <ActionButton
        title={`${session.registered ? 'Top up' : 'Fund'} session (${sol(SESSION_FUNDING_LAMPORTS)} SOL)`}
        busy={fund.isPending}
        disabled={busy}
        onPress={() => fund.mutate()}
      />
    )
  } else if (!session.registered) {
    action = (
      <ActionButton
        title="Register sender"
        busy={register.isPending}
        disabled={busy}
        onPress={() => register.mutate()}
      />
    )
  }

  return (
    <View style={styles.panel}>
      <Text style={styles.detail}>
        Session {ellipsify(session.pubkey)} · {sol(session.balanceLamports)} SOL ·{' '}
        {session.registered ? 'registered' : 'not registered'}
      </Text>
      {action}
      {lastError ? <Text style={styles.error}>{errorText(lastError)}</Text> : null}
    </View>
  )
}

function ActionButton({
  title,
  onPress,
  busy = false,
  disabled = false,
}: {
  title: string
  onPress: () => void
  busy?: boolean
  disabled?: boolean
}) {
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.button, (pressed || busy) && styles.buttonPressed, disabled && !busy && styles.buttonDisabled]}
    >
      {({ pressed }) =>
        busy ? (
          <ActivityIndicator color="#ffffff" />
        ) : (
          <Text style={[styles.buttonText, pressed && styles.buttonTextPressed]}>{title}</Text>
        )
      }
    </Pressable>
  )
}

const styles = StyleSheet.create({
  panel: {
    gap: 6,
    marginTop: 12,
  },
  detail: {
    color: '#444444',
    fontSize: 13,
  },
  error: {
    color: '#b00020',
    fontSize: 13,
  },
  button: {
    alignItems: 'center',
    borderColor: '#000000',
    borderWidth: 1.5,
    minHeight: 40,
    justifyContent: 'center',
    paddingHorizontal: 12,
  },
  buttonPressed: {
    backgroundColor: '#000000',
  },
  buttonDisabled: {
    opacity: 0.4,
  },
  buttonText: {
    fontSize: 15,
  },
  buttonTextPressed: {
    color: '#ffffff',
  },
})
