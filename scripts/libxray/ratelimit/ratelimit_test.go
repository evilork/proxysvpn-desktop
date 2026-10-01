package ratelimit

import (
	"testing"
	"time"
)

type fakeClock struct {
	t     time.Time
	slept []time.Duration
}

func (c *fakeClock) now() time.Time { return c.t }

func (c *fakeClock) sleep(d time.Duration) {
	c.slept = append(c.slept, d)
	c.t = c.t.Add(d)
}

func newFake(rate float64, capacity int64) (*Bucket, *fakeClock) {
	clock := &fakeClock{t: time.Unix(1_000_000, 0)}
	return newBucket(rate, capacity, clock.now, clock.sleep), clock
}

func TestAFullBucketDoesNotWait(t *testing.T) {
	b, clock := newFake(1000, 500)
	b.Wait(500)
	if len(clock.slept) != 0 {
		t.Fatalf("slept %v with a full bucket", clock.slept)
	}
}

func TestDebtIsRepaidAtTheRate(t *testing.T) {
	b, clock := newFake(1000, 500)
	b.Wait(500)
	b.Wait(250)
	if len(clock.slept) != 1 || clock.slept[0] != 250*time.Millisecond {
		t.Fatalf("want one 250ms sleep, got %v", clock.slept)
	}
}

func TestRefillStopsAtCapacity(t *testing.T) {
	b, clock := newFake(1000, 100)
	b.Wait(100)
	clock.t = clock.t.Add(time.Hour)
	b.Wait(150)
	if len(clock.slept) != 1 || clock.slept[0] != 50*time.Millisecond {
		t.Fatalf("an hour idle must refill to 100 tokens only, slept %v", clock.slept)
	}
}

func TestNonPositiveCountTakesNothing(t *testing.T) {
	b, clock := newFake(10, 10)
	b.Wait(0)
	b.Wait(-5)
	b.Wait(10)
	if len(clock.slept) != 0 {
		t.Fatalf("slept %v", clock.slept)
	}
}

func TestInvalidParametersPanic(t *testing.T) {
	for _, tc := range []struct {
		rate     float64
		capacity int64
	}{{0, 1}, {-1, 1}, {1, 0}, {1, -1}} {
		func() {
			defer func() {
				if recover() == nil {
					t.Fatalf("rate %v capacity %v did not panic", tc.rate, tc.capacity)
				}
			}()
			NewBucketWithRate(tc.rate, tc.capacity)
		}()
	}
}

func TestRealClockWaitIsBounded(t *testing.T) {
	b := NewBucketWithRate(1_000_000, 1)
	start := time.Now()
	b.Wait(1)
	b.Wait(1000)
	if elapsed := time.Since(start); elapsed > time.Second {
		t.Fatalf("1000 tokens at 1e6/s took %v", elapsed)
	}
}
