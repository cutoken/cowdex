package main

import (
	"bufio"
	"context"
	"crypto/hmac"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

//go:embed index.html
var page []byte

//go:embed templates.json
var templateData []byte
var templates []map[string]any
var version = "dev"

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "Could not open vault manager:", err)
		os.Exit(1)
	}
}
func run() error {
	serve := flag.Bool("serve", false, "Run in the foreground")
	port := flag.Int("port", 2749, "Local browser port (0 selects a free port with --serve)")
	ttl := flag.Int64("ttl", 86400, "Unlock duration in seconds, up to 86400")
	noBrowser := flag.Bool("no-browser", false, "Print the manager URL without opening a browser")
	showVersion := flag.Bool("version", false, "Print the vault executable version")
	flag.Parse()
	if *showVersion {
		fmt.Println("Cowdex Vault " + version)
		return nil
	}
	if *ttl < 1 || *ttl > 86400 || *port < 0 || *port > 65535 || (*port == 0 && !*serve) {
		return fmt.Errorf("use --ttl 1–86400 and a valid local port")
	}
	home := os.Getenv("COWDEX_VAULT_HOME")
	if home == "" {
		user, err := os.UserHomeDir()
		if err != nil {
			return err
		}
		home = filepath.Join(user, ".local", "share", "cowdex-vault")
	}
	if !filepath.IsAbs(home) {
		return fmt.Errorf("COWDEX_VAULT_HOME must be absolute")
	}
	if err := os.MkdirAll(home, 0700); err != nil {
		return err
	}
	info, err := os.Lstat(home)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return refused
	}
	if err = os.Chmod(home, 0700); err != nil {
		return err
	}
	address := fmt.Sprintf("127.0.0.1:%d", *port)
	if !*serve {
		var child *exec.Cmd
		for attempt := 0; attempt < 50; attempt++ {
			if prove(home, address) {
				url := "http://" + address
				fmt.Println("Vault manager:", url)
				if !*noBrowser {
					openBrowser(url)
				}
				return nil
			}
			if attempt == 0 {
				executable, err := os.Executable()
				if err != nil {
					return err
				}
				child = exec.Command(executable, "--serve", "--port", fmt.Sprint(*port), "--ttl", fmt.Sprint(*ttl))
				if err = startDetached(child); err != nil {
					return err
				}
			}
			time.Sleep(100 * time.Millisecond)
		}
		return fmt.Errorf("selected port is unavailable or manager could not start")
	}
	if err := harden(); err != nil {
		return err
	}

	unlock, err := lockHome(home)
	if err != nil {
		return fmt.Errorf("a vault manager is already using this directory")
	}
	defer unlock()
	if json.Unmarshal(templateData, &templates) != nil {
		return refused
	}
	listener, err := net.Listen("tcp4", address)
	if err != nil {
		return err
	}
	defer listener.Close()
	authority := listener.Addr().String()
	endpoint := filepath.Join(home, "manager.sock")
	network := "unix"
	if runtime.GOOS == "windows" {
		network = "tcp4"
		endpoint = "127.0.0.1:0"
	} else if info, err := os.Lstat(endpoint); err == nil {
		if info.Mode()&os.ModeSocket == 0 {
			return refused
		}
		old, err := net.DialTimeout("unix", endpoint, time.Second)
		if err == nil {
			old.Close()
			return fmt.Errorf("vault manager is already running")
		}
		if err = os.Remove(endpoint); err != nil {
			return err
		}
	}
	broker, err := net.Listen(network, endpoint)
	if err != nil {
		return err
	}
	defer broker.Close()
	if network == "unix" {
		if err = os.Chmod(endpoint, 0600); err != nil {
			return err
		}
		defer os.Remove(endpoint)
	} else {
		endpoint = "tcp://" + broker.Addr().String()
	}
	manager := newManager(home, endpoint, *ttl)
	defer func() { clear(manager.key) }()
	defer os.Remove(filepath.Join(home, "grant.json"))
	proof := random(32)
	if err = atomicWrite(filepath.Join(home, "manager.proof"), proof); err != nil {
		return err
	}
	server := &http.Server{ReadTimeout: 5 * time.Second, WriteTimeout: 30 * time.Second, IdleTimeout: 30 * time.Second}
	manager.stop = func() { _ = server.Close(); _ = broker.Close() }
	server.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { manager.serveHTTP(authority, proof, w, r) })
	go func() {
		for {
			conn, err := broker.Accept()
			if err != nil {
				return
			}
			go manager.handleSocket(conn)
		}
	}()
	done := make(chan struct{})
	defer close(done)
	go func() {
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-done:
				return
			case <-ticker.C:
				manager.mu.Lock()
				expired := manager.expires > 0 && time.Now().Unix() >= manager.expires
				manager.mu.Unlock()
				if expired {
					manager.stop()
					return
				}
			}
		}
	}()
	fmt.Println("Vault manager: http://" + authority)
	err = server.Serve(listener)
	if err == http.ErrServerClosed {
		return nil
	}
	return err
}
func prove(home, address string) bool {
	proof, err := readPrivate(filepath.Join(home, "manager.proof"))
	if err != nil {
		return false
	}
	challenge := hex.EncodeToString(random(32))
	client := http.Client{Timeout: time.Second}
	response, err := client.Get("http://" + address + "/api/proof?challenge=" + challenge)
	if err != nil {
		return false
	}
	defer response.Body.Close()
	var result map[string]string
	if response.StatusCode != 200 || boundedJSON(response.Body, &result) != nil {
		return false
	}
	mac := hmac.New(sha256.New, proof)
	mac.Write([]byte(challenge))
	return hmac.Equal([]byte(result["proof"]), []byte(hex.EncodeToString(mac.Sum(nil))))
}
func openBrowser(url string) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", url)
	case "windows":
		cmd = exec.Command("rundll32.exe", "url.dll,FileProtocolHandler", url)
	default:
		cmd = exec.Command("xdg-open", url)
	}
	_ = cmd.Start()
}
func (m *Manager) handleSocket(conn net.Conn) {
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(30 * time.Second))
	line, err := bufio.NewReader(io.LimitReader(conn, maxMessage+1)).ReadBytes('\n')
	var request map[string]json.RawMessage
	var result any
	if err == nil && len(line) <= maxMessage && json.Unmarshal(line, &request) == nil && request != nil {
		ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
		defer cancel()
		result, err = m.dispatch(ctx, request)
	} else {
		err = refused
	}
	if err != nil {
		result = map[string]string{"error": "Vault operation refused."}
	}
	_ = json.NewEncoder(conn).Encode(result)
}
func (m *Manager) serveHTTP(authority string, proof []byte, w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("X-Frame-Options", "DENY")
	w.Header().Set("X-Cowdex-Vault", "1")
	w.Header().Set("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
	reply := func(status int, value any) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(status)
		_ = json.NewEncoder(w).Encode(value)
	}
	deny := func() {
		reply(403, map[string]string{"error": "Vault operation refused. Check your password and fields."})
	}
	if r.Host != authority {
		deny()
		return
	}
	if r.Method == "GET" && r.URL.Path == "/" {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(page)
		return
	}
	if r.Method == "GET" && r.URL.Path == "/api/proof" {
		challenge := r.URL.Query().Get("challenge")
		raw, err := hex.DecodeString(challenge)
		if err != nil || len(raw) != 32 {
			deny()
			return
		}
		mac := hmac.New(sha256.New, proof)
		mac.Write([]byte(challenge))
		reply(200, map[string]string{"proof": hex.EncodeToString(mac.Sum(nil))})
		return
	}
	token := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
	m.mu.Lock()
	defer m.mu.Unlock()
	if r.Method == "GET" {
		switch r.URL.Path {
		case "/api/state":
			reply(200, m.state(token))
		case "/api/approvals":
			if !m.authorized(token) {
				deny()
				return
			}
			m.purge()
			reply(200, m.approvals())
		default:
			reply(404, map[string]string{"error": "Not found."})
		}
		return
	}
	if r.Method != "POST" || r.Header.Get("Origin") != "http://"+authority || r.Header.Get("Content-Type") != "application/json" || len(r.TransferEncoding) > 0 {
		deny()
		return
	}
	var body map[string]any
	if boundedJSON(r.Body, &body) != nil || body == nil {
		deny()
		return
	}
	var result any
	var err error
	if r.URL.Path == "/api/unlock" {
		result, err = m.unlock(body)
	} else {
		if !m.authorized(token) {
			deny()
			return
		}
		m.purge()
		switch r.URL.Path {
		case "/api/save":
			result, err = m.save(body)
		case "/api/approve", "/api/deny":
			found := false
			for claim, p := range m.pending {
				if p.Code == body["code"] {
					found = true
					if r.URL.Path == "/api/approve" {
						if p.Approved == 0 {
							p.Approved = min(time.Now().Unix()+86400, m.expires)
						}
					} else {
						delete(m.pending, claim)
					}
					break
				}
			}
			if !found {
				err = refused
			}
			result = m.approvals()
		case "/api/lock":
			result = map[string]any{"unlocked": false}
			clear(m.key)
			m.key, m.admin, m.ticket = nil, "", ""
			m.credentials = Credentials{}
			m.pending, m.sessions = map[string]*Pending{}, map[string]*Session{}
			m.expires = 0
			go func() { time.Sleep(100 * time.Millisecond); m.stop() }()
		default:
			err = refused
		}
	}
	if err != nil {
		deny()
		return
	}
	reply(200, result)
}
