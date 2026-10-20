package main

import (
	"os/exec"
	"syscall"
)

func startDetached(cmd *exec.Cmd) error {
	cmd.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x00000008 | 0x00000200}
	return cmd.Start()
}
