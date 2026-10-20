package main

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"runtime"

	"golang.org/x/crypto/scrypt"
)

var refused = errors.New("Vault operation refused.")

const maxVault = 1048576
const maxMessage = 65536
const maxResponse = 8192

type Credential struct {
	Origin     string   `json:"origin"`
	Prefix     string   `json:"prefix"`
	Methods    []string `json:"methods"`
	Token      string   `json:"token"`
	Username   string   `json:"username,omitempty"`
	Auth       string   `json:"auth,omitempty"`
	BodyFormat string   `json:"body_format,omitempty"`
	Template   string   `json:"template,omitempty"`
}
type Credentials map[string]Credential

func random(n int) []byte {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		panic(err)
	}
	return b
}
func ticket() string { return base64.RawURLEncoding.EncodeToString(random(32)) }

func readPrivate(path string) ([]byte, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() || info.Size() > maxVault || runtime.GOOS != "windows" && info.Mode().Perm()&0077 != 0 {
		return nil, refused
	}
	return os.ReadFile(path)
}

func atomicWrite(path string, raw []byte) error {
	if len(raw) > maxVault {
		return refused
	}
	if info, err := os.Lstat(path); err == nil && !info.Mode().IsRegular() {
		return refused
	} else if err != nil && !os.IsNotExist(err) {
		return err
	}
	file, err := os.CreateTemp(filepath.Dir(path), ".vault-*")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	if err = file.Chmod(0600); err == nil {
		_, err = file.Write(raw)
	}
	if err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	return os.Rename(file.Name(), path)
}

func fileKey(raw []byte, password string) ([]byte, error) {
	if len(raw) < 48 || string(raw[:4]) != "CV01" {
		return nil, refused
	}
	return scrypt.Key([]byte(password), raw[4:20], 32768, 8, 1, 32)
}
func decrypt(raw, key []byte) (Credentials, error) {
	if len(raw) < 48 || string(raw[:4]) != "CV01" {
		return nil, refused
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, refused
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	plain, err := aead.Open(nil, raw[20:32], raw[32:], []byte("cowdex-vault-v1"))
	if err != nil {
		return nil, refused
	}
	defer clear(plain)
	var credentials Credentials
	if json.Unmarshal(plain, &credentials) != nil || credentials == nil {
		return nil, refused
	}
	for alias, c := range credentials {
		if c.Auth == "" {
			c.Auth = "bearer"
		}
		if c.BodyFormat == "" {
			c.BodyFormat = "json"
		}
		if c.Template == "" {
			c.Template = "custom"
		}
		if validate(alias, c) != nil {
			return nil, refused
		}
		credentials[alias] = c
	}
	return credentials, nil
}
func encrypt(credentials Credentials, key, salt []byte) ([]byte, error) {
	plain, err := json.Marshal(credentials)
	if err != nil {
		return nil, err
	}
	defer clear(plain)
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	nonce := random(12)
	raw := append(append([]byte("CV01"), salt...), nonce...)
	return aead.Seal(raw, nonce, plain, []byte("cowdex-vault-v1")), nil
}

func boundedJSON(reader io.Reader, value any) error {
	raw, err := io.ReadAll(io.LimitReader(reader, maxMessage+1))
	if err != nil || len(raw) > maxMessage || json.Unmarshal(raw, value) != nil {
		return refused
	}
	return nil
}
