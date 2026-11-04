import { StyleSheet, View } from 'react-native'
import React from 'react'

const MIN_HEIGHT = 0.15
/** Speech RMS rarely exceeds ~0.25; scale it up so bars use the full height. */
const GAIN = 4

/** Rolling audio levels from the mic while talking, or the speaker while receiving. */
export function TalkFeatureWaveform({ levels }: { levels: number[] }) {
  return (
    <View style={styles.row}>
      {levels.map((level, index) => (
        <View
          key={index}
          style={[styles.bar, { height: `${(MIN_HEIGHT + Math.min(1, level * GAIN) * (1 - MIN_HEIGHT)) * 100}%` }]}
        />
      ))}
    </View>
  )
}

const styles = StyleSheet.create({
  row: {
    alignItems: 'flex-end',
    flexDirection: 'row',
    gap: 4,
    height: 72,
    justifyContent: 'center',
  },
  bar: {
    borderColor: '#000000',
    borderWidth: 1.5,
    width: 8,
  },
})
