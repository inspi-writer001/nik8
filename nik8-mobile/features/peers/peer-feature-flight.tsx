import { StyleSheet, View } from 'react-native'
import Animated, {
  Easing,
  useAnimatedStyle,
  useSharedValue,
  withDelay,
  withTiming,
} from 'react-native-reanimated'
import { scheduleOnRN } from 'react-native-worklets'
import React, { useEffect } from 'react'
import { Rect } from '@/features/peers/peer-feature-field'

export interface Flight {
  id: number
  /** Window coordinates of the tapped bubble and of the Push button. */
  from: Rect
  to: Rect
}

const RIPPLE_MS = 450
const FLY_DELAY_MS = 60
const FLY_MS = 420
const BURST_MS = 160

/**
 * Overlay that animates a selected peer bubble: a ripple at the bubble, then the
 * bubble flies to the Push button, grows into it and bursts. `origin` is the window
 * position of the overlay's container, to convert window coordinates.
 */
export function PeerFeatureFlight({
  flight,
  origin,
  onDone,
}: {
  flight: Flight | null
  origin: { x: number; y: number }
  onDone: () => void
}) {
  return (
    <View pointerEvents="none" style={StyleSheet.absoluteFill}>
      {flight ? <FlyingBubble key={flight.id} flight={flight} origin={origin} onDone={onDone} /> : null}
    </View>
  )
}

function FlyingBubble({ flight, origin, onDone }: { flight: Flight; origin: { x: number; y: number }; onDone: () => void }) {
  const { from, to } = flight
  const size = from.width
  const start = { x: from.x - origin.x + from.width / 2, y: from.y - origin.y + from.height / 2 }
  const end = { x: to.x - origin.x + to.width / 2, y: to.y - origin.y + to.height / 2 }
  const grow = to.width / size

  const ripple = useSharedValue(0)
  const travel = useSharedValue(0)
  const burst = useSharedValue(0)

  useEffect(() => {
    ripple.value = withTiming(1, { duration: RIPPLE_MS, easing: Easing.out(Easing.quad) })
    travel.value = withDelay(
      FLY_DELAY_MS,
      withTiming(1, { duration: FLY_MS, easing: Easing.inOut(Easing.cubic) }, (finished) => {
        if (!finished) return
        burst.value = withTiming(1, { duration: BURST_MS, easing: Easing.out(Easing.quad) }, (done) => {
          if (done) scheduleOnRN(onDone)
        })
      }),
    )
  }, [ripple, travel, burst, onDone])

  const rippleStyle = useAnimatedStyle(() => ({
    opacity: 0.45 * (1 - ripple.value),
    transform: [{ scale: 1 + ripple.value * 1.4 }],
  }))

  const bubbleStyle = useAnimatedStyle(() => {
    // A gentle arc: lift a little mid-flight so it doesn't slide in a straight line.
    const arc = Math.sin(travel.value * Math.PI) * -24
    return {
      opacity: 1 - burst.value,
      transform: [
        { translateX: (end.x - start.x) * travel.value },
        { translateY: (end.y - start.y) * travel.value + arc },
        { scale: (1 + (grow - 1) * travel.value) * (1 + burst.value * 0.25) },
      ],
    }
  })

  const circle = { borderRadius: size / 2, height: size, left: start.x - size / 2, top: start.y - size / 2, width: size }

  return (
    <>
      <Animated.View style={[styles.ripple, circle, rippleStyle]} />
      <Animated.View style={[styles.bubble, circle, bubbleStyle]} />
    </>
  )
}

const styles = StyleSheet.create({
  ripple: {
    borderColor: '#000000',
    borderWidth: 2,
    position: 'absolute',
  },
  bubble: {
    backgroundColor: '#000000',
    position: 'absolute',
  },
})
