package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"mime/multipart"
	"net/http"
	"net/url"
	"regexp"
	"slices"
	"strings"
	"time"
	"unicode/utf8"
)

var aliasPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{1,32}$`)
var methods = []string{"DELETE", "GET", "HEAD", "PATCH", "POST", "PUT"}

func canonical(path string) (string, error) {
	if !strings.HasPrefix(path, "/") || strings.HasPrefix(path, "//") || strings.ContainsAny(path, "\\#") {
		return "", refused
	}
	for _, c := range path {
		if c < 33 || c > 126 {
			return "", refused
		}
	}
	route, _, _ := strings.Cut(path, "?")
	if strings.Contains(route, "%") || strings.Contains(route, "//") {
		return "", refused
	}
	for _, part := range strings.Split(route, "/") {
		if part == "." || part == ".." {
			return "", refused
		}
	}
	return route, nil
}
func printable(value string, min int) bool {
	if len(value) < min || len(value) > 4096 {
		return false
	}
	for _, c := range value {
		if c < 33 || c > 126 {
			return false
		}
	}
	return true
}
func validate(alias string, c Credential) error {
	origin, err := url.Parse(c.Origin)
	if !aliasPattern.MatchString(alias) || err != nil || origin.Scheme != "https" || origin.Hostname() == "" || origin.User != nil || origin.Path != "" || origin.RawQuery != "" || origin.Fragment != "" || (origin.Port() != "" && origin.Port() != "443") {
		return refused
	}
	if !printable(c.Token, 8) || !slices.Contains([]string{"", "bearer", "basic", "helio"}, c.Auth) || !slices.Contains([]string{"", "json", "form", "multipart"}, c.BodyFormat) {
		return refused
	}
	if (c.Auth == "basic" || c.Auth == "helio") && (!printable(c.Username, 1) || strings.Contains(c.Username, ":")) {
		return refused
	}
	if len(c.Methods) == 0 {
		return refused
	}
	for _, method := range c.Methods {
		if !slices.Contains(methods, method) {
			return refused
		}
	}
	route, err := canonical(c.Prefix)
	if err != nil || route != c.Prefix {
		return refused
	}
	return nil
}
func policy(c Credential) string {
	c.Token = ""
	c.Template = ""
	c.Methods = slices.Clone(c.Methods)
	slices.Sort(c.Methods)
	raw, _ := json.Marshal(c)
	return string(raw)
}

var upstreamClient = &http.Client{
	Timeout:       20 * time.Second,
	CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	Transport:     &http.Transport{Proxy: http.ProxyFromEnvironment, DisableCompression: true, TLSHandshakeTimeout: 10 * time.Second},
}

func upstream(ctx context.Context, c Credential, method, path string, body *string) (any, error) {
	if validate("request", c) != nil {
		return nil, refused
	}
	auth := "Bearer " + c.Token
	redactions := []string{c.Token}
	if c.Auth == "basic" {
		encoded := base64.StdEncoding.EncodeToString([]byte(c.Username + ":" + c.Token))
		auth = "Basic " + encoded
		redactions = append([]string{encoded}, redactions...)
	}
	if c.Auth == "helio" {
		route, q, _ := strings.Cut(path, "?")
		query, err := url.ParseQuery(q)
		if err != nil {
			return nil, refused
		}
		query.Set("apiKey", c.Username)
		path = route + "?" + query.Encode()
		redactions = append(redactions, c.Username)
	}
	var reader io.Reader
	contentType := "application/json"
	if body != nil {
		payload := []byte(*body)
		if c.BodyFormat == "form" || c.BodyFormat == "multipart" {
			var fields map[string]any
			if json.Unmarshal(payload, &fields) != nil || fields == nil {
				return nil, refused
			}
			values := url.Values{}
			for key, value := range fields {
				items, ok := value.([]any)
				if !ok {
					items = []any{value}
				}
				for _, item := range items {
					var text string
					switch v := item.(type) {
					case string:
						text = v
					case float64, bool:
						raw, _ := json.Marshal(v)
						text = string(raw)
					default:
						return nil, refused
					}
					values.Add(key, text)
				}
			}
			payload = []byte(values.Encode())
			contentType = "application/x-www-form-urlencoded"
			if c.BodyFormat == "multipart" {
				var buf bytes.Buffer
				writer := multipart.NewWriter(&buf)
				for key, items := range values {
					if strings.ContainsAny(key, "\r\n\"\\") {
						return nil, refused
					}
					for _, value := range items {
						if writer.WriteField(key, value) != nil {
							return nil, refused
						}
					}
				}
				if writer.Close() != nil {
					return nil, refused
				}
				payload = buf.Bytes()
				contentType = writer.FormDataContentType()
			}
		}
		reader = bytes.NewReader(payload)
	}
	req, err := http.NewRequestWithContext(ctx, method, c.Origin+path, reader)
	if err != nil {
		return nil, refused
	}
	req.Header.Set("Authorization", auth)
	req.Header.Set("Accept-Encoding", "identity")
	req.Header.Set("User-Agent", "Cowdex-Vault")
	if body != nil {
		req.Header.Set("Content-Type", contentType)
	}
	response, err := upstreamClient.Do(req)
	if err != nil {
		return nil, refused
	}
	defer response.Body.Close()
	if response.StatusCode >= 300 && response.StatusCode < 400 || (response.Header.Get("Content-Encoding") != "" && response.Header.Get("Content-Encoding") != "identity") {
		return nil, refused
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, maxResponse+1))
	if err != nil || len(raw) > maxResponse {
		return nil, refused
	}
	for _, secret := range redactions {
		raw = bytes.ReplaceAll(raw, []byte(secret), []byte("[REDACTED]"))
	}
	if len(raw) > maxResponse {
		return nil, refused
	}
	text := strings.ToValidUTF8(string(raw), string(utf8.RuneError))
	return map[string]any{"status": response.StatusCode, "body": text, "truncated": false}, nil
}
