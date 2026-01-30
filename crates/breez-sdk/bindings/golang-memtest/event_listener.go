package main

import (
	"fmt"
	"sync"
	"sync/atomic"

	sdk "breez_sdk_spark_go/breez_sdk_spark"
)

// TestEventListener implements sdk.EventListener and tracks events.
type TestEventListener struct {
	name string

	mu           sync.Mutex
	eventCounts  map[string]int64
	totalEvents  int64
	lastEvent    sdk.SdkEvent
	paymentsChan chan sdk.SdkEvent
}

// NewTestEventListener creates a new test event listener.
func NewTestEventListener(name string) *TestEventListener {
	return &TestEventListener{
		name:         name,
		eventCounts:  make(map[string]int64),
		paymentsChan: make(chan sdk.SdkEvent, 100),
	}
}

// OnEvent implements sdk.EventListener.
func (l *TestEventListener) OnEvent(event sdk.SdkEvent) {
	l.mu.Lock()
	defer l.mu.Unlock()

	l.totalEvents++
	eventType := getEventType(event)
	l.eventCounts[eventType]++
	l.lastEvent = event

	// Forward payment events to channel for synchronization
	switch event.(type) {
	case *sdk.SdkEventPaymentSucceeded, *sdk.SdkEventPaymentFailed, *sdk.SdkEventPaymentPending:
		select {
		case l.paymentsChan <- event:
		default:
			// Channel full, drop event (test will continue)
		}
	}
}

// GetEventCounts returns a copy of event counts.
func (l *TestEventListener) GetEventCounts() map[string]int64 {
	l.mu.Lock()
	defer l.mu.Unlock()
	result := make(map[string]int64)
	for k, v := range l.eventCounts {
		result[k] = v
	}
	return result
}

// GetTotalEvents returns the total event count.
func (l *TestEventListener) GetTotalEvents() int64 {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.totalEvents
}

// PaymentsChan returns the channel for payment events.
func (l *TestEventListener) PaymentsChan() <-chan sdk.SdkEvent {
	return l.paymentsChan
}

// PrintStats prints event statistics.
func (l *TestEventListener) PrintStats() {
	l.mu.Lock()
	defer l.mu.Unlock()

	fmt.Printf("Event stats for %s:\n", l.name)
	fmt.Printf("  Total events: %d\n", l.totalEvents)
	for eventType, count := range l.eventCounts {
		fmt.Printf("  %s: %d\n", eventType, count)
	}
}

// getEventType returns a string representation of the event type.
func getEventType(event sdk.SdkEvent) string {
	switch event.(type) {
	case *sdk.SdkEventSynced:
		return "Synced"
	case *sdk.SdkEventPaymentSucceeded:
		return "PaymentSucceeded"
	case *sdk.SdkEventPaymentFailed:
		return "PaymentFailed"
	case *sdk.SdkEventPaymentPending:
		return "PaymentPending"
	case *sdk.SdkEventUnclaimedDeposits:
		return "UnclaimedDeposits"
	case *sdk.SdkEventClaimedDeposits:
		return "ClaimedDeposits"
	case *sdk.SdkEventOptimization:
		return "Optimization"
	default:
		return "Unknown"
	}
}

// ListenerManager manages multiple event listeners for churn testing.
type ListenerManager struct {
	mu          sync.Mutex
	sdk         *sdk.BreezSdk
	listenerIDs []string
	listeners   []*TestEventListener
	counter     int64
}

// NewListenerManager creates a new listener manager.
func NewListenerManager(sdkInstance *sdk.BreezSdk) *ListenerManager {
	return &ListenerManager{
		sdk:         sdkInstance,
		listenerIDs: make([]string, 0),
		listeners:   make([]*TestEventListener, 0),
	}
}

// AddListeners adds n new listeners.
func (m *ListenerManager) AddListeners(n int) {
	m.mu.Lock()
	defer m.mu.Unlock()

	for i := 0; i < n; i++ {
		name := fmt.Sprintf("listener-%d", atomic.AddInt64(&m.counter, 1))
		listener := NewTestEventListener(name)
		id := m.sdk.AddEventListener(listener)
		m.listenerIDs = append(m.listenerIDs, id)
		m.listeners = append(m.listeners, listener)
	}
}

// RemoveListeners removes up to n listeners.
func (m *ListenerManager) RemoveListeners(n int) int {
	m.mu.Lock()
	defer m.mu.Unlock()

	removed := 0
	for i := 0; i < n && len(m.listenerIDs) > 0; i++ {
		id := m.listenerIDs[0]
		m.listenerIDs = m.listenerIDs[1:]
		m.listeners = m.listeners[1:]
		m.sdk.RemoveEventListener(id)
		removed++
	}
	return removed
}

// Count returns the current number of listeners.
func (m *ListenerManager) Count() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return len(m.listenerIDs)
}

// RemoveAll removes all listeners.
func (m *ListenerManager) RemoveAll() {
	m.mu.Lock()
	defer m.mu.Unlock()

	for _, id := range m.listenerIDs {
		m.sdk.RemoveEventListener(id)
	}
	m.listenerIDs = nil
	m.listeners = nil
}
