import { Pressable, StyleSheet, Text, View } from 'react-native'
import Animated, { Easing, useAnimatedStyle, useSharedValue, withSequence, withTiming } from 'react-native-reanimated'
import * as Haptics from 'expo-haptics'
import React, { Ref, useEffect } from 'react'

export function TalkFeaturePushButton({
  disabled = false,
  pulse = 0,
  buttonRef,
  onPressIn,
  onPressOut,
}: {
  disabled?: boolean
  /** Changing this plays a short "absorb" pulse, e.g. after a peer bubble lands on it. */
  pulse?: number
  buttonRef?: Ref<View>
  onPressIn: () => void
  onPressOut: () => void
}) {
  const scale = useSharedValue(1)

  useEffect(() => {
    if (pulse === 0) return
    scale.value = withSequence(
      withTiming(1.06, { duration: 120, easing: Easing.out(Easing.quad) }),
      withTiming(1, { duration: 220, easing: Easing.inOut(Easing.quad) }),
    )
  }, [pulse, scale])

  const pulseStyle = useAnimatedStyle(() => ({ transform: [{ scale: scale.value }] }))

  return (
    <Animated.View ref={buttonRef} style={pulseStyle}>
      <Pressable
        accessibilityLabel="Push to talk"
        accessibilityRole="button"
        accessibilityState={{ disabled }}
        disabled={disabled}
        onPressIn={() => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium)
          onPressIn()
        }}
        onPressOut={() => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light)
          onPressOut()
        }}
        style={({ pressed }) => [styles.button, pressed && styles.buttonPressed, disabled && styles.buttonDisabled]}
      >
        {({ pressed }) => <Text style={[styles.label, pressed && styles.labelPressed]}>Push</Text>}
      </Pressable>
    </Animated.View>
  )
}

export const PUSH_BUTTON_SIZE = 200

const styles = StyleSheet.create({
  button: {
    alignItems: 'center',
    borderColor: '#000000',
    borderRadius: PUSH_BUTTON_SIZE / 2,
    borderWidth: 2,
    height: PUSH_BUTTON_SIZE,
    justifyContent: 'center',
    width: PUSH_BUTTON_SIZE,
  },
  buttonPressed: {
    backgroundColor: '#000000',
    transform: [{ scale: 0.97 }],
  },
  buttonDisabled: {
    opacity: 0.3,
  },
  label: {
    color: '#000000',
    fontSize: 28,
  },
  labelPressed: {
    color: '#ffffff',
  },
})
