// Package ratelimit stands in for github.com/juju/ratelimit in the Apple
// tunnel engine build (scripts/build-libxray.sh replaces the module with this
// directory).
//
// XTLS/REALITY imports juju/ratelimit, which is LGPL-3.0 (with a static
// linking exception), for one thing: slowing down the fallback connection of
// a REALITY SERVER. A client never runs that code, but Go links it anyway, and
// the App Store extension is meant to carry no GPL-family code at all. So this
// module provides the exact API REALITY uses - Bucket, NewBucketWithRate and
// Bucket.Wait - as a plain token bucket of our own, MIT like the rest of this
// repository. It is not a copy of the original.
package ratelimit

import (
	"sync"
	"time"
)

// Bucket is a token bucket that refills continuously at a fixed rate up to
// its capacity. Taking more than is available puts it into debt, and the
// taker sleeps until the debt would be repaid.
type Bucket struct {
	mu       sync.Mutex
	rate     float64 // tokens per second
	capacity float64
	tokens   float64
	last     time.Time
	now      func() time.Time
	sleep    func(time.Duration)
}

// NewBucketWithRate returns a full bucket that refills at rate tokens per
// second and holds at most capacity tokens. Like the original it panics on a
// non-positive rate or capacity: both are programming errors of the caller.
func NewBucketWithRate(rate float64, capacity int64) *Bucket {
	return newBucket(rate, capacity, time.Now, time.Sleep)
}

func newBucket(rate float64, capacity int64, now func() time.Time, sleep func(time.Duration)) *Bucket {
	if rate <= 0 {
		panic("ratelimit: rate must be positive")
	}
	if capacity <= 0 {
		panic("ratelimit: capacity must be positive")
	}
	return &Bucket{
		rate:     rate,
		capacity: float64(capacity),
		tokens:   float64(capacity),
		last:     now(),
		now:      now,
		sleep:    sleep,
	}
}

// Wait takes count tokens, sleeping until the bucket could have supplied
// them. A non-positive count takes nothing and returns at once.
func (b *Bucket) Wait(count int64) {
	if count <= 0 {
		return
	}
	b.mu.Lock()
	now := b.now()
	elapsed := now.Sub(b.last).Seconds()
	if elapsed > 0 {
		b.tokens += elapsed * b.rate
		if b.tokens > b.capacity {
			b.tokens = b.capacity
		}
	}
	b.last = now
	b.tokens -= float64(count)
	var wait time.Duration
	if b.tokens < 0 {
		wait = time.Duration(-b.tokens / b.rate * float64(time.Second))
	}
	b.mu.Unlock()
	if wait > 0 {
		b.sleep(wait)
	}
}
