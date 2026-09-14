export type PreviewPointerLikeEvent = {
  button: number
  clientX: number
  clientY: number
  timeStamp: number
}

const RIGHT_MOUSE_BUTTON = 2
const DRAG_THRESHOLD_PX = 6
const CLICK_DURATION_MS = 500

type RightPointerState = {
  startX: number
  startY: number
  startTime: number
  dragged: boolean
}

export class PreviewContextMenuGesture {
  private rightPointer: RightPointerState | null = null
  private lastRightPointer: RightPointerState | null = null

  pointerDown(event: PreviewPointerLikeEvent) {
    if (event.button !== RIGHT_MOUSE_BUTTON) {
      this.rightPointer = null
      this.lastRightPointer = null
      return
    }

    this.rightPointer = {
      startX: event.clientX,
      startY: event.clientY,
      startTime: event.timeStamp,
      dragged: false,
    }
    this.lastRightPointer = null
  }

  pointerMove(event: PreviewPointerLikeEvent) {
    if (!this.rightPointer) return
    const deltaX = event.clientX - this.rightPointer.startX
    const deltaY = event.clientY - this.rightPointer.startY
    if (Math.hypot(deltaX, deltaY) > DRAG_THRESHOLD_PX) {
      this.rightPointer.dragged = true
    }
  }

  pointerUp(event: PreviewPointerLikeEvent) {
    if (!this.rightPointer || event.button !== RIGHT_MOUSE_BUTTON) return
    this.lastRightPointer = this.rightPointer
    this.rightPointer = null
  }

  pointerCancel() {
    this.rightPointer = null
    this.lastRightPointer = null
  }

  shouldOpen(event: PreviewPointerLikeEvent) {
    const gesture = this.lastRightPointer || this.rightPointer
    this.lastRightPointer = null
    this.rightPointer = null

    if (!gesture) return false
    if (gesture.dragged) return false
    return event.timeStamp - gesture.startTime <= CLICK_DURATION_MS
  }
}
