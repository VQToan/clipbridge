package service

import "time"

// SetNowForTest allows tests in service_test to override the clock.
func (r *Relay) SetNowForTest(now func() time.Time) {
	r.now = now
}
