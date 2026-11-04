import { Pressable, StyleSheet, Text, useWindowDimensions, View } from 'react-native'
import Animated, {
  Easing,
  LinearTransition,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
  ZoomIn,
  ZoomOut,
} from 'react-native-reanimated'
import { Address } from '@solana/kit'
import React, { useEffect, useRef } from 'react'
import { ellipsify } from '@/utils/ellipsify'
import { DiscoveryStatus, Peer } from '@/features/peers/use-peer-discovery'

export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** The field opens to this share of the screen height once comms are online. */
const FIELD_SHARE = 0.2
const OPEN_MS = 320

/** Signal-strength distance is rough; show it as such. */
export function formatDistance(meters?: number): string {
  if (meters === undefined) return 'nearby'
  if (meters < 1) return `~${meters.toFixed(1)}m`
  if (meters < 15) return `~${Math.round(meters)}m`
  return 'far'
}

function emptyLabel(status: DiscoveryStatus): string {
  if (status === 'denied') return 'Allow Nearby devices to find peers'
  if (typeof status === 'object') return status.error
  return 'Searching for peers…'
}

/** Nearer peers get bigger bubbles. */
export function bubbleSize(meters?: number) {
  if (meters === undefined) return 56
  return Math.max(40, Math.min(72, 72 - meters * 5))
}

/**
 * Nearby peers as bubbles, inline in the status box. Opens when `open` (comms online).
 * Tapping a bubble reports it with its on-screen rect so the caller can animate it.
 */
export function PeerFeatureField({
  open,
  status,
  peers,
  selected,
  onSelect,
}: {
  open: boolean
  status: DiscoveryStatus
  peers: Peer[]
  selected: Address | null
  onSelect: (peer: Address | null, from: Rect | null) => void
}) {
  const { height: windowHeight } = useWindowDimensions()
  const reduceMotion = useReducedMotion()
  const height = useSharedValue(0)
  const fieldHeight = Math.round(windowHeight * FIELD_SHARE)

  useEffect(() => {
    const target = open ? fieldHeight : 0
    height.value = reduceMotion
      ? target
      : withTiming(target, { duration: OPEN_MS, easing: Easing.out(Easing.cubic) })
  }, [open, fieldHeight, reduceMotion, height])

  const fieldStyle = useAnimatedStyle(() => ({ height: height.value, opacity: height.value / fieldHeight }))

  return (
    <Animated.View style={[styles.field, fieldStyle]}>
      {peers.length === 0 ? (
        <Text style={styles.empty}>{emptyLabel(status)}</Text>
      ) : (
        <View style={styles.bubbles}>
          {peers.map((peer) => (
            <PeerBubble
              key={peer.address}
              peer={peer}
              selected={peer.address === selected}
              reduceMotion={reduceMotion}
              onPress={(from) => onSelect(peer.address === selected ? null : peer.address, from)}
            />
          ))}
        </View>
      )}
    </Animated.View>
  )
}

function PeerBubble({
  peer,
  selected,
  reduceMotion,
  onPress,
}: {
  peer: Peer
  selected: boolean
  reduceMotion: boolean
  onPress: (from: Rect | null) => void
}) {
  const circle = useRef<View>(null)
  const size = bubbleSize(peer.distanceMeters)

  return (
    <Animated.View
      entering={reduceMotion ? undefined : ZoomIn.springify().damping(14)}
      exiting={reduceMotion ? undefined : ZoomOut.duration(180)}
      layout={reduceMotion ? undefined : LinearTransition.springify().damping(18)}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${selected ? 'Disconnect from' : 'Connect to'} peer ${peer.address}, ${formatDistance(peer.distanceMeters)}`}
        accessibilityState={{ selected }}
        hitSlop={8}
        onPress={() => {
          if (!circle.current) return onPress(null)
          circle.current.measureInWindow((x, y, width, height) => onPress({ x, y, width, height }))
        }}
        style={styles.peer}
      >
        <View
          ref={circle}
          style={[styles.bubble, { borderRadius: size / 2, height: size, width: size }, selected && styles.selected]}
        />
        <Text style={styles.distance}>{formatDistance(peer.distanceMeters)}</Text>
        <Text style={styles.address}>{ellipsify(peer.address)}</Text>
      </Pressable>
    </Animated.View>
  )
}

const styles = StyleSheet.create({
  field: {
    justifyContent: 'center',
    overflow: 'hidden',
  },
  empty: {
    color: '#444444',
    fontSize: 13,
    textAlign: 'center',
  },
  bubbles: {
    alignItems: 'flex-end',
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 20,
    justifyContent: 'center',
  },
  peer: {
    alignItems: 'center',
    gap: 2,
  },
  bubble: {
    borderColor: '#000000',
    borderWidth: 1.5,
  },
  selected: {
    backgroundColor: '#000000',
  },
  distance: {
    fontSize: 13,
  },
  address: {
    color: '#666666',
    fontSize: 10,
  },
})
