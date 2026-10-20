//go:build !windows

package main

import (
	"os"
	"path/filepath"
	"syscall"
)

func lockHome(home string) (func(), error) {
	file, err := os.OpenFile(filepath.Join(home, "manager.lock"), os.O_CREATE|os.O_RDWR|syscall.O_NOFOLLOW, 0600)
	if err != nil {
		return nil, err
	}
	if err = syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		file.Close()
		return nil, refused
	}
	return func() { _ = syscall.Flock(int(file.Fd()), syscall.LOCK_UN); _ = file.Close() }, nil
}
