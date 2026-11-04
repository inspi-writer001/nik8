import { Pressable, StyleSheet, Text, View } from 'react-native'
import { Address } from '@solana/kit'
import React, { ReactNode } from 'react'
import Nik8Native from '@/modules/nik8-native'
import { useNetwork } from '@/features/network/use-network'
import { ListenerStats } from '@/modules/nik8-native'
import { CommsStatus } from '@/features/talk/use-nik8-listener'
import { ellipsify } from '@/utils/ellipsify'

function statusLabel(status: CommsStatus, transmitting: boolean, receiving: boolean): string {
  switch (status.state) {
    case 'connecting':
      return 'Connecting…'
    case 'online':
      return transmitting ? 'Transmitting' : receiving ? 'Receiving' : 'Online'
    case 'reconnecting':
      return `Reconnecting… (attempt ${status.attempt})`
    case 'failed':
      return 'Offline'
    case 'unconfigured':
      return 'Not configured'
  }
}

/** Connection state for `address` (this device's session key), plus `children` (the peer field). */
export function TalkFeatureStatus({
  address,
  status,
  stats,
  retry,
  transmitting,
  receiving,
  children,
}: {
  address: Address
  status: CommsStatus
  stats: ListenerStats | null
  retry: () => void
  transmitting: boolean
  receiving: boolean
  children?: ReactNode
}) {
  const { selectedNetwork } = useNetwork()

  return (
    <View style={styles.box}>
      <Text style={styles.state}>{statusLabel(status, transmitting, receiving)}</Text>
      {status.state === 'failed' ? (
        <>
          <Text style={styles.detail}>{status.reason}</Text>
          <Pressable accessibilityRole="button" onPress={retry}>
            <Text style={styles.retry}>Tap to retry</Text>
          </Pressable>
        </>
      ) : null}
      {status.state === 'reconnecting' ? <Text style={styles.detail}>{status.reason}</Text> : null}
      {status.state === 'unconfigured' ? (
        <Text style={styles.detail}>Set EXPO_PUBLIC_NIK8_GRPC_ENDPOINT in .env</Text>
      ) : null}
      {stats && stats.accepted > 0 ? (
        <Text style={styles.detail}>
          Received {stats.accepted} packets · last seq {stats.lastSequence}
        </Text>
      ) : null}
      {children}
      <Text style={styles.meta}>
        You {ellipsify(address)} · {selectedNetwork.label} · core v{Nik8Native.version()}
      </Text>
    </View>
  )
}

const styles = StyleSheet.create({
  box: {
    borderColor: '#000000',
    borderWidth: 1.5,
    gap: 4,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  state: {
    fontSize: 18,
  },
  detail: {
    color: '#444444',
    fontSize: 13,
  },
  retry: {
    fontSize: 14,
    textDecorationLine: 'underline',
  },
  meta: {
    color: '#666666',
    fontSize: 12,
  },
})
