"""Memory-only attachment requests awaiting an authenticated browser decision."""

from dataclasses import dataclass


def policy(credential):
    return (credential["origin"], credential["prefix"],
            tuple(sorted(credential["methods"])), credential.get("auth", "bearer"),
            credential.get("username"), credential.get("body_format", "json"))


@dataclass(repr=False)
class Pending:
    claim_token: str
    code: str
    policies: dict
    expires_at: int
    approved_until: int | None = None

    def public(self):
        return {"code": self.code, "aliases": sorted(self.policies),
                "expiresAt": self.expires_at, "approved": self.approved_until is not None}

    def response(self):
        return {"claim_token": self.claim_token, "approval_code": self.code,
                "aliases": sorted(self.policies), "expires_at": self.expires_at}
