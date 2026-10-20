package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

func fixture(t *testing.T) (*Manager, *httptest.Server) {
	t.Helper()
	if err := json.Unmarshal(templateData, &templates); err != nil {
		t.Fatal(err)
	}
	m := newManager(t.TempDir(), "tcp://127.0.0.1:45678", 86400)
	server := httptest.NewUnstartedServer(nil)
	authority := server.Listener.Addr().String()
	server.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { m.serveHTTP(authority, []byte("fake-proof"), w, r) })
	server.Start()
	m.stop = server.Close
	t.Cleanup(server.Close)
	return m, server
}
func api(t *testing.T, server *httptest.Server, method, path, token string, body any) (int, map[string]any) {
	t.Helper()
	raw, _ := json.Marshal(body)
	req, err := http.NewRequest(method, server.URL+path, bytes.NewReader(raw))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Origin", server.URL)
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	response, err := server.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	var result map[string]any
	if err = json.NewDecoder(response.Body).Decode(&result); err != nil {
		t.Fatal(err)
	}
	return response.StatusCode, result
}
func operation(t *testing.T, m *Manager, body any) (map[string]any, error) {
	t.Helper()
	raw, _ := json.Marshal(body)
	var request map[string]json.RawMessage
	_ = json.Unmarshal(raw, &request)
	result, err := m.dispatch(context.Background(), request)
	if err != nil {
		return nil, err
	}
	raw, _ = json.Marshal(result)
	var decoded map[string]any
	_ = json.Unmarshal(raw, &decoded)
	return decoded, nil
}
func unlockFixture(t *testing.T, s *httptest.Server) string {
	t.Helper()
	status, result := api(t, s, "POST", "/api/unlock", "", map[string]any{"password": "fake-password", "confirmPassword": "fake-password"})
	if status != 200 {
		t.Fatal("unlock failed", result)
	}
	return result["adminToken"].(string)
}
func saveFixture(t *testing.T, s *httptest.Server, admin, token, prefix string) {
	t.Helper()
	status, result := api(t, s, "POST", "/api/save", admin, map[string]any{"alias": "cf", "template": "cloudflare", "token": token, "prefix": prefix, "methods": []string{"GET", "POST"}})
	if status != 200 {
		t.Fatal("save failed", result)
	}
}
func attachFixture(t *testing.T, m *Manager, s *httptest.Server, admin string) string {
	t.Helper()
	pending, err := operation(t, m, map[string]any{"op": "attach", "ticket": m.ticket, "scope": []string{"cf"}})
	if err != nil {
		t.Fatal(err)
	}
	claim, err := operation(t, m, map[string]any{"op": "claim", "claim_token": pending["claim_token"]})
	if err != nil || claim["approval_code"] != pending["approval_code"] {
		t.Fatal("claim must wait for approval")
	}
	status, _ := api(t, s, "POST", "/api/approve", admin, map[string]any{"code": pending["approval_code"]})
	if status != 200 {
		t.Fatal("approval failed")
	}
	claim, err = operation(t, m, map[string]any{"op": "claim", "claim_token": pending["claim_token"]})
	if err != nil {
		t.Fatal(err)
	}
	return claim["session"].(string)
}

func TestBrowserApprovalRequestRotationAndRevocation(t *testing.T) {
	m, s := fixture(t)
	admin := unlockFixture(t, s)
	saveFixture(t, s, admin, "fake-secret-old", "/client/v4")
	session := attachFixture(t, m, s, admin)
	original := upstreamClient
	defer func() { upstreamClient = original }()
	calls := 0
	upstreamClient = &http.Client{Transport: roundTrip(func(r *http.Request) (*http.Response, error) {
		calls++
		if r.Header.Get("Authorization") != "Bearer fake-secret-new" {
			t.Fatal("rotation not applied")
		}
		return &http.Response{StatusCode: 200, Header: http.Header{}, Body: io.NopCloser(strings.NewReader("echo fake-secret-new"))}, nil
	})}
	saveFixture(t, s, admin, "fake-secret-new", "/client/v4")
	request := map[string]any{"op": "request", "session": session, "credential": "cf", "method": "GET", "path": "/client/v4/accounts"}
	result, err := operation(t, m, request)
	if err != nil || result["body"] != "echo [REDACTED]" {
		t.Fatal("redacted request failed", result, err)
	}
	request["path"] = "/client/v40/accounts"
	if _, err = operation(t, m, request); err == nil {
		t.Fatal("prefix escape accepted")
	}
	saveFixture(t, s, admin, "", "/client/v4/accounts")
	request["path"] = "/client/v4/accounts"
	if _, err = operation(t, m, request); err == nil {
		t.Fatal("policy edit must revoke session")
	}
	if calls != 1 {
		t.Fatal("rejected requests reached upstream")
	}
	status, state := api(t, s, "GET", "/api/state", admin, nil)
	if status != 200 {
		t.Fatal("state failed")
	}
	raw, _ := json.Marshal(state)
	if bytes.Contains(raw, []byte("fake-secret")) {
		t.Fatal("state exposed saved credential")
	}
}
func TestEncryptedVaultReopensWithoutPlaintext(t *testing.T) {
	m, s := fixture(t)
	admin := unlockFixture(t, s)
	saveFixture(t, s, admin, "fake-secret-token", "/")
	raw, err := readPrivate(filepath.Join(m.home, "vault.enc"))
	if err != nil || bytes.Contains(raw, []byte("fake-secret-token")) {
		t.Fatal("encrypted storage failed")
	}
	key, err := fileKey(raw, "fake-password")
	if err != nil {
		t.Fatal(err)
	}
	reopened, err := decrypt(raw, key)
	if err != nil || !reflect.DeepEqual(reopened, m.credentials) {
		t.Fatal("vault reopen failed")
	}
	wrong, _ := fileKey(raw, "wrong-password")
	if _, err = decrypt(raw, wrong); err == nil {
		t.Fatal("wrong password accepted")
	}
	raw[len(raw)-1] ^= 1
	if _, err = decrypt(raw, key); err == nil {
		t.Fatal("tampered ciphertext accepted")
	}
}
func TestBrowserRejectsCrossOriginAndUnauthenticatedChanges(t *testing.T) {
	_, s := fixture(t)
	unlockFixture(t, s)
	status, _ := api(t, s, "POST", "/api/save", "", map[string]any{})
	if status != 403 {
		t.Fatal("missing browser authorization accepted")
	}
	req, _ := http.NewRequest("POST", s.URL+"/api/unlock", strings.NewReader(`{"password":"fake-password"}`))
	req.Header.Set("Origin", "https://example.org")
	req.Header.Set("Content-Type", "application/json")
	response, err := s.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if response.StatusCode != 403 {
		t.Fatal("foreign origin accepted")
	}
	req, _ = http.NewRequest("GET", s.URL+"/api/state", nil)
	req.Host = "example.org"
	response, err = s.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if response.StatusCode != 403 {
		t.Fatal("foreign host accepted")
	}
}
func TestAttachmentExpiresAndDetachRevokes(t *testing.T) {
	m, s := fixture(t)
	admin := unlockFixture(t, s)
	saveFixture(t, s, admin, "fake-secret-token", "/")
	session := attachFixture(t, m, s, admin)
	if _, err := operation(t, m, map[string]any{"op": "detach", "session": session}); err != nil {
		t.Fatal(err)
	}
	if _, err := operation(t, m, map[string]any{"op": "status", "session": session}); err == nil {
		t.Fatal("detached session accepted")
	}
	session = attachFixture(t, m, s, admin)
	m.sessions[session].Expires = time.Now().Unix() - 1
	if _, err := operation(t, m, map[string]any{"op": "status", "session": session}); err == nil {
		t.Fatal("expired session accepted")
	}
	session = attachFixture(t, m, s, admin)
	status, _ := api(t, s, "POST", "/api/lock", admin, map[string]any{})
	if status != 200 {
		t.Fatal("lock failed")
	}
	if _, err := operation(t, m, map[string]any{"op": "status", "session": session}); err == nil {
		t.Fatal("locked vault still authorizes attachments")
	}
}
func TestSocketProtocolFramingAndResponse(t *testing.T) {
	m, s := fixture(t)
	admin := unlockFixture(t, s)
	saveFixture(t, s, admin, "fake-secret-token", "/")
	client, server := net.Pipe()
	defer client.Close()
	go m.handleSocket(server)
	if err := json.NewEncoder(client).Encode(map[string]any{"op": "attach", "ticket": m.ticket, "scope": []string{"cf"}}); err != nil {
		t.Fatal(err)
	}
	var result map[string]any
	if err := json.NewDecoder(client).Decode(&result); err != nil {
		t.Fatal(err)
	}
	if result["approval_code"] == nil {
		t.Fatal("socket attachment failed")
	}
}

type roundTrip func(*http.Request) (*http.Response, error)

func (f roundTrip) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
func TestRequestFormatsAndResponseBounds(t *testing.T) {
	original := upstreamClient
	defer func() { upstreamClient = original }()
	for _, format := range []string{"json", "form", "multipart"} {
		t.Run(format, func(t *testing.T) {
			c := Credential{Origin: "https://example.org", Prefix: "/", Methods: []string{"POST"}, Token: "fake-secret-token", Username: "api", Auth: "basic", BodyFormat: format}
			upstreamClient = &http.Client{Transport: roundTrip(func(r *http.Request) (*http.Response, error) {
				user, password, ok := r.BasicAuth()
				if !ok || user != "api" || password != c.Token {
					t.Fatal("basic authentication failed")
				}
				if format != "json" {
					if strings.HasPrefix(r.Header.Get("Content-Type"), "multipart/") {
						if err := r.ParseMultipartForm(1024); err != nil {
							t.Fatal(err)
						}
					} else if err := r.ParseForm(); err != nil {
						t.Fatal(err)
					}
					if !reflect.DeepEqual(r.Form["to"], []string{"a", "b"}) {
						t.Fatal("repeated fields lost")
					}
				}
				return &http.Response{StatusCode: 200, Header: http.Header{}, Body: io.NopCloser(strings.NewReader("ok"))}, nil
			})}
			body := `{"to":["a","b"],"enabled":true}`
			if _, err := upstream(context.Background(), c, "POST", "/send", &body); err != nil {
				t.Fatal(err)
			}
		})
	}
	c := Credential{Origin: "https://example.org", Prefix: "/", Methods: []string{"GET"}, Token: "fake-secret-token"}
	for _, scenario := range []string{"redirect", "compressed", "oversized"} {
		t.Run(scenario, func(t *testing.T) {
			upstreamClient = &http.Client{Transport: roundTrip(func(*http.Request) (*http.Response, error) {
				response := &http.Response{StatusCode: 200, Header: http.Header{}, Body: io.NopCloser(strings.NewReader("ok"))}
				switch scenario {
				case "redirect":
					response.StatusCode = 302
				case "compressed":
					response.Header.Set("Content-Encoding", "gzip")
				case "oversized":
					response.Body = io.NopCloser(strings.NewReader(strings.Repeat("x", maxResponse+1)))
				}
				return response, nil
			})}
			if _, err := upstream(context.Background(), c, "GET", "/", nil); err == nil {
				t.Fatal("unsafe response accepted")
			}
		})
	}
}
func TestCanonicalPaths(t *testing.T) {
	for _, path := range []string{"//example.org/x", "/../x", "/a/./b", "/a%2fb", "/a#fragment", "/a\\b", "/a b"} {
		if _, err := canonical(path); err == nil {
			t.Errorf("accepted %q", path)
		}
	}
	if route, err := canonical("/v1/items?q=a%20b"); err != nil || route != "/v1/items" {
		t.Fatal("encoded query rejected")
	}
}
