package main

import (
	"encoding/hex"
	"path/filepath"
	"testing"
)

func TestLegacyCredentialDefaultsPreserveTokenOnEdit(t *testing.T) {
	m, server := fixture(t)
	header := append([]byte("CV01"), make([]byte, 44)...)
	key, err := fileKey(header, "fake-password")
	if err != nil {
		t.Fatal(err)
	}
	defer clear(key)
	raw, err := encrypt(Credentials{"cf": {Origin: "https://api.cloudflare.com", Prefix: "/", Methods: []string{"GET"}, Token: "fake-legacy-token"}}, key, header[4:20])
	if err != nil {
		t.Fatal(err)
	}
	if err = atomicWrite(filepath.Join(m.home, "vault.enc"), raw); err != nil {
		t.Fatal(err)
	}
	admin := unlockFixture(t, server)
	status, _ := api(t, server, "POST", "/api/save", admin, map[string]any{"alias": "cf", "template": "custom", "origin": "https://api.cloudflare.com", "auth": "bearer", "body_format": "json", "prefix": "/", "methods": []string{"GET"}, "token": ""})
	if status != 200 || m.credentials["cf"].Token != "fake-legacy-token" {
		t.Fatal("legacy credential edit lost its saved token")
	}
}

func TestOnlyOneManagerCanUseAVaultDirectory(t *testing.T) {
	home := t.TempDir()
	unlock, err := lockHome(home)
	if err != nil {
		t.Fatal(err)
	}
	if second, err := lockHome(home); err == nil {
		second()
		unlock()
		t.Fatal("second manager accepted")
	}
	unlock()
	unlock, err = lockHome(home)
	if err != nil {
		t.Fatal("released manager lock was not reusable")
	}
	unlock()
}

func TestOpensVaultWrittenByPython(t *testing.T) {
	raw, err := hex.DecodeString("4356303144d80023a48d9fa994aed654d534ed59b40d7d8452f5a94006c4df1a1bb2a8c6090ca6eeb90830536bf9f2bc202a4ce3299e2b641ff0ab4f41e6ebdde22c736517ab738bb5acebb005bf0dd441d884c7d0c15ae9603114b4e27a58ee4840aff8b980c9c4d22cfbd683e0d87e80a70d2ec04c4db4616cb070965b0f6ed8027542b4e3cb687b7fb9d93f26582b915bd29178541d4ed8cafb48c922bdd03f7767b9a9bbe99e91d4ba1ca64f232d26ec502917253d2e01607d60c1fa19dfc48f4897637d7b49928303194d73c9a92fc6f77140495dd8ce90b623f56d385dc4a4f4bb2c74e08081abf799b4")
	if err != nil {
		t.Fatal(err)
	}
	key, err := fileKey(raw, "fake-password")
	if err != nil {
		t.Fatal(err)
	}
	defer clear(key)
	credentials, err := decrypt(raw, key)
	if err != nil {
		t.Fatal("Python vault is not compatible", err)
	}
	c := credentials["cf"]
	if len(credentials) != 1 || c.Token != "fake-python-token" || c.Prefix != "/client/v4" {
		t.Fatal("Python credential did not survive decoding")
	}
	updated, err := encrypt(credentials, key, raw[4:20])
	if err != nil {
		t.Fatal(err)
	}
	if _, err = decrypt(updated, key); err != nil {
		t.Fatal("updating a migrated vault failed")
	}
}
