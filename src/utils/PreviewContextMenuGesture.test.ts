import { describe, expect, test } from 'vitest'
import { PreviewContextMenuGesture } from './PreviewContextMenuGesture'

function event(button: number, clientX: number, clientY: number, timeStamp: number) {
  return { button, clientX, clientY, timeStamp }
}

describe('PreviewContextMenuGesture', () => {
  test('opens menu for a short right-click without movement', () => {
    const gesture = new PreviewContextMenuGesture()

    gesture.pointerDown(event(2, 10, 20, 100))
    gesture.pointerUp(event(2, 10, 20, 140))

    expect(gesture.shouldOpen(event(2, 10, 20, 145))).toBe(true)
  })

  test('suppresses menu after right-button drag', () => {
    const gesture = new PreviewContextMenuGesture()

    gesture.pointerDown(event(2, 10, 20, 100))
    gesture.pointerMove(event(2, 30, 20, 140))
    gesture.pointerUp(event(2, 30, 20, 180))

    expect(gesture.shouldOpen(event(2, 30, 20, 185))).toBe(false)
  })

  test('suppresses menu after long right-button hold', () => {
    const gesture = new PreviewContextMenuGesture()

    gesture.pointerDown(event(2, 10, 20, 100))
    gesture.pointerUp(event(2, 10, 20, 720))

    expect(gesture.shouldOpen(event(2, 10, 20, 725))).toBe(false)
  })

  test('ignores non-right-button gestures', () => {
    const gesture = new PreviewContextMenuGesture()

    gesture.pointerDown(event(0, 10, 20, 100))
    gesture.pointerUp(event(0, 10, 20, 140))

    expect(gesture.shouldOpen(event(0, 10, 20, 145))).toBe(false)
  })
})
