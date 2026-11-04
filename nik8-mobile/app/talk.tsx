import { StyleSheet, Text, View } from 'react-native'
import { useReducedMotion } from 'react-native-reanimated'
import { SafeAreaView } from 'react-native-safe-area-context'
import { Redirect } from 'expo-router'
import { useMobileWallet } from '@wallet-ui/react-native-kit'
import { Address, address } from '@solana/kit'
import React, { useCallback, useRef, useState } from 'react'
import { TalkFeatureStatus } from '@/features/talk/talk-feature-status'
import { TalkFeatureWaveform } from '@/features/talk/talk-feature-waveform'
import { TalkFeaturePushButton } from '@/features/talk/talk-feature-push-button'
import { SessionFeaturePanel } from '@/features/session/session-feature-panel'
import { useSession } from '@/features/session/use-session'
import { useTalk } from '@/features/talk/use-talk'
import { useNik8Listener } from '@/features/talk/use-nik8-listener'
import { usePeerDiscovery } from '@/features/peers/use-peer-discovery'
import { formatDistance, PeerFeatureField, Rect } from '@/features/peers/peer-feature-field'
import { Flight, PeerFeatureFlight } from '@/features/peers/peer-feature-flight'
import { ellipsify } from '@/utils/ellipsify'
import Nik8Native from '@/modules/nik8-native'

export default function TalkScreen() {
  const { account } = useMobileWallet()
  const { pubkey } = useSession()

  if (!account) {
    return <Redirect href="/" />
  }
  // Devices are identified by their session key: it's what peers advertise, listen
  // on and address packets to. The wallet only funds it.
  return <Talk self={pubkey.data ? address(pubkey.data) : undefined} />
}

function Talk({ self }: { self: Address | undefined }) {
  const { ready } = useSession()
  const listener = useNik8Listener(self)
  const discovery = usePeerDiscovery(self)
  const reduceMotion = useReducedMotion()
  const [peer, setPeer] = useState<Address | null>(null)
  const [flight, setFlight] = useState<Flight | null>(null)
  const [pulse, setPulse] = useState(0)
  const [origin, setOrigin] = useState({ x: 0, y: 0 })
  const root = useRef<View>(null)
  const pushButton = useRef<View>(null)

  const recipient = peer ?? self
  const talk = useTalk(recipient, ready && !!recipient)
  const selectedPeer = discovery.peers.find((nearby) => nearby.address === peer)

  const select = useCallback(
    (next: Address | null, from: Rect | null) => {
      setPeer(next)
      if (!next || !from || reduceMotion || !pushButton.current) return
      pushButton.current.measureInWindow((x, y, width, height) =>
        setFlight({ id: Date.now(), from, to: { x, y, width, height } }),
      )
    },
    [reduceMotion],
  )

  const landed = useCallback(() => {
    setFlight(null)
    setPulse((count) => count + 1)
  }, [])

  return (
    <View
      ref={root}
      style={styles.root}
      onLayout={() => root.current?.measureInWindow((x, y) => setOrigin({ x, y }))}
    >
      <SafeAreaView style={styles.screen}>
        {self ? (
          <TalkFeatureStatus
            address={self}
            status={listener.status}
            stats={listener.stats}
            retry={listener.retry}
            transmitting={talk.transmitting}
            receiving={talk.receiving}
          >
            <PeerFeatureField
              open={listener.status.state === 'online'}
              status={discovery.status}
              peers={discovery.peers}
              selected={peer}
              onSelect={select}
            />
          </TalkFeatureStatus>
        ) : null}
        <SessionFeaturePanel />
        <View style={styles.waveform}>
          <TalkFeatureWaveform levels={talk.levels} />
        </View>
        <View style={styles.push}>
          <TalkFeaturePushButton
            buttonRef={pushButton}
            disabled={!ready}
            pulse={pulse}
            onPressIn={talk.pressIn}
            onPressOut={talk.pressOut}
          />
          <Text style={styles.hint}>
            {!ready
              ? 'Fund and register the session to talk'
              : peer
                ? `Talking to ${ellipsify(peer)}${selectedPeer ? ` · ${formatDistance(selectedPeer.distanceMeters)}` : ' · out of range'}`
                : 'Tap a peer to connect · talking to yourself'}
            {talk.sentPackets > 0 ? ` · sent ${talk.sentPackets} via ${Nik8Native.submitPath() === 'ingress' ? 'sender' : 'RPC'}` : ''}
          </Text>
          {talk.error ? <Text style={styles.error}>{talk.error}</Text> : null}
        </View>
      </SafeAreaView>
      <PeerFeatureFlight flight={flight} origin={origin} onDone={landed} />
    </View>
  )
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: '#ffffff',
  },
  screen: {
    flex: 1,
    paddingHorizontal: 32,
    paddingTop: 48,
  },
  waveform: {
    marginTop: 32,
  },
  push: {
    alignItems: 'center',
    flex: 1,
    gap: 12,
    justifyContent: 'center',
  },
  hint: {
    color: '#666666',
    fontSize: 13,
    textAlign: 'center',
  },
  error: {
    color: '#b00020',
    fontSize: 13,
    textAlign: 'center',
  },
})
