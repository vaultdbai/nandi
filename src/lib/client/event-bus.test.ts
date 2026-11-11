import { describe, expect, it, vi } from 'vitest'
import { createEventBus } from './event-bus'
import type { NandiEvent } from './events'

const runEvent = { event: 'agent://run', payload: {} } as NandiEvent

/** A bus over a fake transport that records connects and disconnects */
function setup() {
  const transport = {
    connects: 0,
    disconnects: 0,
    emit: vi.fn() as (event: NandiEvent) => void,
  }
  const bus = createEventBus(emit => {
    transport.connects += 1
    transport.emit = emit
    return {
      ready: Promise.resolve(),
      unsubscribe: () => {
        transport.disconnects += 1
      },
    }
  })
  return { bus, transport }
}

describe('createEventBus', () => {
  it('connects the transport once for any number of subscribers', () => {
    const { bus, transport } = setup()
    bus.subscribe(vi.fn())
    bus.subscribe(vi.fn())
    bus.subscribe(vi.fn())
    expect(transport.connects).toBe(1)
  })

  it('delivers each event to every subscriber', () => {
    const { bus, transport } = setup()
    const first = vi.fn()
    const second = vi.fn()
    bus.subscribe(first)
    bus.subscribe(second)

    transport.emit(runEvent)

    expect(first).toHaveBeenCalledExactlyOnceWith(runEvent)
    expect(second).toHaveBeenCalledExactlyOnceWith(runEvent)
  })

  it('disconnects only when the last subscriber leaves', () => {
    const { bus, transport } = setup()
    const first = bus.subscribe(vi.fn())
    const second = bus.subscribe(vi.fn())

    first.unsubscribe()
    expect(transport.disconnects).toBe(0)
    second.unsubscribe()
    expect(transport.disconnects).toBe(1)
  })

  it('stops delivering to a subscriber that left', () => {
    const { bus, transport } = setup()
    const stays = vi.fn()
    const leaves = vi.fn()
    bus.subscribe(stays)
    bus.subscribe(leaves).unsubscribe()

    transport.emit(runEvent)

    expect(stays).toHaveBeenCalledOnce()
    expect(leaves).not.toHaveBeenCalled()
  })

  it('ignores a second unsubscribe from the same subscription', () => {
    const { bus, transport } = setup()
    const first = bus.subscribe(vi.fn())
    bus.subscribe(vi.fn())

    first.unsubscribe()
    first.unsubscribe()

    expect(transport.disconnects).toBe(0)
  })

  it('treats the same function subscribed twice as two subscriptions', () => {
    const { bus, transport } = setup()
    const handler = vi.fn()
    const first = bus.subscribe(handler)
    bus.subscribe(handler)

    first.unsubscribe()
    transport.emit(runEvent)

    expect(handler).toHaveBeenCalledOnce()
  })

  it('connects again when a subscriber arrives after everyone left', () => {
    const { bus, transport } = setup()
    bus.subscribe(vi.fn()).unsubscribe()
    bus.subscribe(vi.fn())

    expect(transport.connects).toBe(2)
    expect(transport.disconnects).toBe(1)
  })

  it('keeps delivering when one handler throws', () => {
    const { bus, transport } = setup()
    const after = vi.fn()
    bus.subscribe(() => {
      throw new Error('boom')
    })
    bus.subscribe(after)

    transport.emit(runEvent)

    expect(after).toHaveBeenCalledOnce()
  })

  it('lets a handler unsubscribe while an event is delivered', () => {
    const { bus, transport } = setup()
    const later = vi.fn()
    const first = bus.subscribe(() => first.unsubscribe())
    bus.subscribe(later)

    transport.emit(runEvent)

    expect(later).toHaveBeenCalledOnce()
  })
})
