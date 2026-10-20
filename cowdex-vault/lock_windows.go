package main

import (
	"os"
	"path/filepath"
	"syscall"
	"unsafe"
)

func lockHome(home string) (func(), error) {
	file, err := os.OpenFile(filepath.Join(home, "manager.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	dll := syscall.NewLazyDLL("kernel32.dll")
	lock := dll.NewProc("LockFileEx")
	unlock := dll.NewProc("UnlockFileEx")
	overlapped := new(syscall.Overlapped)
	result, _, _ := lock.Call(file.Fd(), 0x2|0x1, 0, 1, 0, uintptr(unsafe.Pointer(overlapped)))
	if result == 0 {
		file.Close()
		return nil, refused
	}
	return func() {
		_, _, _ = unlock.Call(file.Fd(), 0, 1, 0, uintptr(unsafe.Pointer(overlapped)))
		_ = file.Close()
	}, nil
}
