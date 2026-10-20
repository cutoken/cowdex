package main

import (
	"context"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

type Pending struct {
	Claim, Code       string
	Policies          map[string]string
	Expires, Approved int64
}
type Session struct {
	Policies map[string]string
	Expires  int64
}
type Manager struct {
	mu                            sync.Mutex
	home, endpoint, admin, ticket string
	key, salt                     []byte
	credentials                   Credentials
	pending                       map[string]*Pending
	sessions                      map[string]*Session
	expires, nextAttempt          int64
	ttl                           int64
	stop                          func()
}

func equal(a, b string) bool { return a != "" && subtle.ConstantTimeCompare([]byte(a), []byte(b)) == 1 }
func newManager(home, endpoint string, ttl int64) *Manager {
	return &Manager{home: home, endpoint: endpoint, ttl: ttl, credentials: Credentials{}, pending: map[string]*Pending{}, sessions: map[string]*Session{}}
}
func (m *Manager) authorized(token string) bool {
	return len(m.key) > 0 && time.Now().Unix() < m.expires && equal(token, m.admin)
}
func (m *Manager) purge() {
	now := time.Now().Unix()
	for k, p := range m.pending {
		if now >= p.Expires || p.Approved > 0 && now >= p.Approved {
			delete(m.pending, k)
		}
	}
	for k, s := range m.sessions {
		if now >= s.Expires {
			delete(m.sessions, k)
		}
	}
}
func (m *Manager) publish() error {
	aliases := make([]string, 0, len(m.credentials))
	for alias := range m.credentials {
		aliases = append(aliases, alias)
	}
	slices.Sort(aliases)
	if len(aliases) == 0 {
		m.ticket = ""
		err := os.Remove(filepath.Join(m.home, "grant.json"))
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	m.ticket = ticket()
	if len(aliases) > 8 {
		aliases = aliases[:8]
	}
	raw, _ := json.Marshal(map[string]any{"socket": m.endpoint, "ticket": m.ticket, "scope": aliases})
	return atomicWrite(filepath.Join(m.home, "grant.json"), raw)
}
func (m *Manager) approvals() map[string]any {
	list := []any{}
	for _, p := range m.pending {
		list = append(list, map[string]any{"code": p.Code, "aliases": policyAliases(p.Policies), "expiresAt": p.Expires, "approved": p.Approved > 0})
	}
	return map[string]any{"approvals": list}
}
func (m *Manager) state(token string) map[string]any {
	_, err := os.Stat(filepath.Join(m.home, "vault.enc"))
	state := map[string]any{"unlocked": false, "exists": err == nil, "templates": templates, "approvalsRequired": true}
	if !m.authorized(token) {
		return state
	}
	m.purge()
	state["unlocked"] = true
	state["expiresAt"] = m.expires
	state["approvals"] = m.approvals()["approvals"]
	state["ready"] = m.ticket != ""
	entries := []any{}
	for _, alias := range credentialAliases(m.credentials) {
		c := m.credentials[alias]
		entries = append(entries, map[string]any{"alias": alias, "origin": c.Origin, "prefix": c.Prefix, "methods": c.Methods, "template": c.Template, "auth": c.Auth, "body_format": c.BodyFormat, "hasUsername": c.Username != ""})
	}
	state["credentials"] = entries
	return state
}
func credentialAliases(c Credentials) []string {
	aliases := make([]string, 0, len(c))
	for a := range c {
		aliases = append(aliases, a)
	}
	slices.Sort(aliases)
	return aliases
}
func policyAliases(p map[string]string) []string {
	aliases := make([]string, 0, len(p))
	for a := range p {
		aliases = append(aliases, a)
	}
	slices.Sort(aliases)
	return aliases
}
func (m *Manager) unlock(body map[string]any) (any, error) {
	now := time.Now().Unix()
	if now < m.nextAttempt {
		return nil, refused
	}
	m.nextAttempt = now + 1
	password, ok := body["password"].(string)
	if !ok || len(password) < 1 || len(password) > 1024 {
		return nil, refused
	}
	path := filepath.Join(m.home, "vault.enc")
	raw, err := readPrivate(path)
	if os.IsNotExist(err) {
		if password != body["confirmPassword"] {
			return nil, refused
		}
		salt := random(16)
		header := append([]byte("CV01"), salt...)
		header = append(header, make([]byte, 28)...)
		key, e := fileKey(header, password)
		if e != nil {
			return nil, e
		}
		raw, e = encrypt(Credentials{}, key, salt)
		clear(key)
		if e != nil {
			return nil, e
		}
		err = atomicWrite(path, raw)
	}
	if err != nil {
		return nil, refused
	}
	key, err := fileKey(raw, password)
	if err != nil {
		return nil, refused
	}
	credentials, err := decrypt(raw, key)
	if err != nil {
		clear(key)
		return nil, refused
	}
	clear(m.key)
	m.key, m.salt, m.credentials = key, slices.Clone(raw[4:20]), credentials
	m.admin = ticket()
	m.expires = now + m.ttl
	m.pending = map[string]*Pending{}
	m.sessions = map[string]*Session{}
	if err = m.publish(); err != nil {
		return nil, err
	}
	result := m.state(m.admin)
	result["adminToken"] = m.admin
	return result, nil
}
func (m *Manager) save(body map[string]any) (any, error) {
	alias, _ := body["alias"].(string)
	templateID, _ := body["template"].(string)
	if templateID == "" {
		templateID = "custom"
	}
	var preset map[string]any
	for _, t := range templates {
		if t["id"] == templateID {
			preset = t
		}
	}
	if preset == nil {
		return nil, refused
	}
	c := Credential{Template: templateID, Prefix: "/", Methods: slices.Clone(methods), Auth: "bearer", BodyFormat: "json"}
	if p, ok := body["prefix"].(string); ok {
		c.Prefix = p
	}
	if entries, ok := body["methods"].([]any); ok {
		c.Methods = nil
		for _, entry := range entries {
			s, ok := entry.(string)
			if !ok {
				return nil, refused
			}
			c.Methods = append(c.Methods, s)
		}
	}
	source := preset
	if templateID == "custom" {
		source = body
	}
	c.Origin, _ = source["origin"].(string)
	if s, ok := source["auth"].(string); ok {
		c.Auth = s
	}
	if s, ok := source["body_format"].(string); ok {
		c.BodyFormat = s
	}
	previous, exists := m.credentials[alias]
	same := exists && previous.Origin == c.Origin && previous.Auth == c.Auth && previous.BodyFormat == c.BodyFormat && previous.Template == c.Template
	c.Token, _ = body["token"].(string)
	c.Username, _ = body["username"].(string)
	if same && c.Token == "" {
		c.Token = previous.Token
	}
	if same && c.Username == "" {
		c.Username = previous.Username
	}
	if username, ok := preset["fixed_username"].(string); ok {
		c.Username = username
	}
	if validate(alias, c) != nil || !exists && len(m.credentials) >= 8 {
		return nil, refused
	}
	updated := Credentials{}
	for a, v := range m.credentials {
		updated[a] = v
	}
	updated[alias] = c
	raw, err := encrypt(updated, m.key, m.salt)
	if err != nil {
		return nil, err
	}
	if atomicWrite(filepath.Join(m.home, "vault.enc"), raw) != nil {
		return nil, refused
	}
	if exists && policy(previous) != policy(c) {
		for k, p := range m.pending {
			if _, ok := p.Policies[alias]; ok {
				delete(m.pending, k)
			}
		}
		for k, s := range m.sessions {
			if _, ok := s.Policies[alias]; ok {
				delete(m.sessions, k)
			}
		}
	}
	m.credentials = updated
	if err = m.publish(); err != nil {
		return nil, err
	}
	return m.state(m.admin), nil
}

func (p *Pending) response() any {
	return map[string]any{"claim_token": p.Claim, "approval_code": p.Code, "aliases": policyAliases(p.Policies), "expires_at": p.Expires}
}
func (m *Manager) dispatch(ctx context.Context, request map[string]json.RawMessage) (any, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	now := time.Now().Unix()
	if len(m.key) == 0 || now >= m.expires {
		return nil, refused
	}
	m.purge()
	str := func(key string) string { var value string; _ = json.Unmarshal(request[key], &value); return value }
	op := str("op")
	switch op {
	case "attach":
		var scope []string
		if json.Unmarshal(request["scope"], &scope) != nil || len(scope) < 1 || len(scope) > 8 || !equal(str("ticket"), m.ticket) || len(m.pending) >= 8 || len(m.sessions) >= 64 {
			return nil, refused
		}
		policies := map[string]string{}
		for _, alias := range scope {
			c, ok := m.credentials[alias]
			if !ok {
				return nil, refused
			}
			policies[alias] = policy(c)
		}
		if len(policies) != len(scope) {
			return nil, refused
		}
		p := &Pending{Claim: ticket(), Code: strings.ToUpper(hex.EncodeToString(random(6))), Policies: policies, Expires: min(now+300, m.expires)}
		m.pending[p.Claim] = p
		if err := m.publish(); err != nil {
			return nil, err
		}
		return p.response(), nil
	case "claim", "cancel":
		claim := str("claim_token")
		p := m.pending[claim]
		if op == "cancel" {
			delete(m.pending, claim)
			return map[string]any{}, nil
		}
		if p == nil {
			return nil, refused
		}
		for alias, frozen := range p.Policies {
			c, ok := m.credentials[alias]
			if !ok || policy(c) != frozen {
				delete(m.pending, claim)
				return nil, refused
			}
		}
		if p.Approved == 0 {
			return p.response(), nil
		}
		if len(m.sessions) >= 64 {
			return nil, refused
		}
		session := ticket()
		m.sessions[session] = &Session{Policies: p.Policies, Expires: p.Approved}
		delete(m.pending, claim)
		return map[string]any{"session": session, "aliases": policyAliases(p.Policies), "expires_at": p.Approved}, nil
	}
	sessionID := str("session")
	s := m.sessions[sessionID]
	if s == nil {
		return nil, refused
	}
	switch op {
	case "status":
		return map[string]any{"aliases": policyAliases(s.Policies), "expires_at": s.Expires}, nil
	case "detach":
		delete(m.sessions, sessionID)
		return map[string]any{}, nil
	case "request":
		for key := range request {
			if !slices.Contains([]string{"op", "session", "credential", "method", "path", "body"}, key) {
				return nil, refused
			}
		}
		alias, method, path := str("credential"), str("method"), str("path")
		frozen, ok := s.Policies[alias]
		c, exists := m.credentials[alias]
		if !ok || !exists || policy(c) != frozen || !slices.Contains(c.Methods, method) {
			return nil, refused
		}
		route, err := canonical(path)
		prefix := strings.TrimRight(c.Prefix, "/")
		if err != nil || route != prefix && !strings.HasPrefix(route, prefix+"/") {
			return nil, refused
		}
		var body *string
		if raw, ok := request["body"]; ok && json.Unmarshal(raw, &body) != nil {
			return nil, refused
		}
		m.mu.Unlock()
		result, err := upstream(ctx, c, method, path, body)
		m.mu.Lock()
		if m.sessions[sessionID] != s || time.Now().Unix() >= s.Expires {
			return nil, refused
		}
		return result, err
	default:
		return nil, refused
	}
}
