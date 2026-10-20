//go:build !windows

package main

import (
	"os/exec"
	"syscall"
)

func startDetached(cmd *exec.Cmd) error {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	return cmd.Start()
}
