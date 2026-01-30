//go:build darwin

package main

import "syscall"

type syscallRusage = syscall.Rusage

func getRusage(rusage *syscallRusage) error {
	return syscall.Getrusage(syscall.RUSAGE_SELF, rusage)
}
