import { useEffect, useState } from 'react'
import { PermissionsAndroid, Platform } from 'react-native'
import { Address, getAddressDecoder, getAddressEncoder, getBase16Encoder } from '@solana/kit'
import Nik8Native from '@/modules/nik8-native'
import { useAppActive } from '@/utils/use-app-active'

export interface Peer {
  address: Address
  rssi: number
  distanceMeters?: number
}

export type DiscoveryStatus = 'idle' | 'searching' | 'denied' | { error: string }

async function ensureBluetoothPermissions(): Promise<boolean> {
  const permissions =
    Platform.OS === 'android' && Platform.Version >= 31
      ? [PermissionsAndroid.PERMISSIONS.BLUETOOTH_SCAN, PermissionsAndroid.PERMISSIONS.BLUETOOTH_ADVERTISE]
      : [PermissionsAndroid.PERMISSIONS.ACCESS_FINE_LOCATION]
  const results = await PermissionsAndroid.requestMultiple(permissions)
  return permissions.every((permission) => results[permission] === PermissionsAndroid.RESULTS.GRANTED)
}

const hex = getBase16Encoder()
const addresses = getAddressDecoder()

/**
 * Advertises `self` over BLE and lists nearby communik8 devices while in the foreground.
 * Peers are sorted strongest signal first.
 */
export function usePeerDiscovery(self: Address | undefined) {
  const active = useAppActive()
  const [status, setStatus] = useState<DiscoveryStatus>('idle')
  const [peers, setPeers] = useState<Peer[]>([])

  useEffect(() => {
    if (!active || !self) return
    let cancelled = false

    const subscriptions = [
      Nik8Native.addListener('onPeers', ({ peers }) =>
        setPeers(
          peers
            .map((peer) => ({ ...peer, address: addresses.decode(hex.encode(peer.address)) }))
            .filter((peer) => peer.address !== self),
        ),
      ),
      Nik8Native.addListener('onDiscoveryError', ({ reason }) => setStatus({ error: reason })),
    ]

    ensureBluetoothPermissions().then((granted) => {
      if (cancelled) return
      if (!granted) {
        setStatus('denied')
        return
      }
      try {
        Nik8Native.startDiscovery(new Uint8Array(getAddressEncoder().encode(self)))
        setStatus('searching')
      } catch (e) {
        setStatus({ error: e instanceof Error ? e.message : String(e) })
      }
    })

    return () => {
      cancelled = true
      subscriptions.forEach((subscription) => subscription.remove())
      Nik8Native.stopDiscovery()
      setPeers([])
    }
  }, [active, self])

  return { status, peers }
}
