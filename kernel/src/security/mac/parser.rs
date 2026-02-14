//! MAC Policy Parser
//!
//! Tokenizes and parses text-based MAC policy language into structured rules,
//! transitions, roles, and user-role mappings. Zero-allocation design using
//! fixed-size arrays and `&'static str` references.

use super::{
    DomainTransition, Permission, PolicyAction, PolicyRule, Role, UserRoleEntry, MAX_PARSED_ROLES,
    MAX_PARSED_RULES, MAX_PARSED_TRANSITIONS, MAX_PARSED_USER_ROLES, MAX_PERMISSIONS,
    MAX_ROLE_TYPES, MAX_TOKENS, MAX_USER_ASSIGNED_ROLES,
};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Token Types
// ---------------------------------------------------------------------------

/// Token types for the policy language.
///
/// Uses `&'static str` references into the input text to avoid allocations.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    /// Keyword: allow, deny, type_transition, role, user, sensitivity, category
    Keyword(&'a str),
    /// Identifier (type names, role names, etc.)
    Ident(&'a str),
    /// Opening brace
    LBrace,
    /// Closing brace
    RBrace,
    /// Colon separator
    Colon,
    /// Semicolon terminator
    Semicolon,
}

// ---------------------------------------------------------------------------
// Parsed Policy Result
// ---------------------------------------------------------------------------

/// Result of parsing a MAC policy text (zero-allocation).
///
/// Contains fixed-size arrays of rules, transitions, roles, and user-role
/// mappings.
pub(super) struct ParsedPolicy {
    pub(super) rules: [Option<PolicyRule>; MAX_PARSED_RULES],
    pub(super) rule_count: usize,
    pub(super) transitions: [Option<DomainTransition>; MAX_PARSED_TRANSITIONS],
    pub(super) transition_count: usize,
    pub(super) roles: [Option<Role>; MAX_PARSED_ROLES],
    pub(super) role_count: usize,
    pub(super) user_roles: [Option<UserRoleEntry>; MAX_PARSED_USER_ROLES],
    pub(super) user_role_count: usize,
}

impl ParsedPolicy {
    pub(super) const fn new() -> Self {
        Self {
            rules: [const { None }; MAX_PARSED_RULES],
            rule_count: 0,
            transitions: [const { None }; MAX_PARSED_TRANSITIONS],
            transition_count: 0,
            roles: [const { None }; MAX_PARSED_ROLES],
            role_count: 0,
            user_roles: [const { None }; MAX_PARSED_USER_ROLES],
            user_role_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Policy Parser
// ---------------------------------------------------------------------------

/// Policy parser that tokenizes and parses policy text without allocations.
pub struct PolicyParser;

impl PolicyParser {
    /// Tokenize a policy string into tokens (zero-allocation).
    ///
    /// Returns references into the input string for identifiers and keywords.
    fn tokenize<'a>(input: &'a str) -> ([Token<'a>; MAX_TOKENS], usize) {
        let mut tokens = [Token::Semicolon; MAX_TOKENS]; // placeholder init
        let mut count = 0;
        let bytes = input.as_bytes();
        let mut pos = 0;

        while pos < bytes.len() && count < MAX_TOKENS {
            let ch = bytes[pos];
            match ch {
                // Skip whitespace
                b' ' | b'\t' | b'\n' | b'\r' => {
                    pos += 1;
                }
                // Skip comments (# to end of line)
                b'#' => {
                    pos += 1;
                    while pos < bytes.len() && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                    if pos < bytes.len() {
                        pos += 1; // skip the newline
                    }
                }
                b'{' => {
                    tokens[count] = Token::LBrace;
                    count += 1;
                    pos += 1;
                }
                b'}' => {
                    tokens[count] = Token::RBrace;
                    count += 1;
                    pos += 1;
                }
                b':' => {
                    tokens[count] = Token::Colon;
                    count += 1;
                    pos += 1;
                }
                b';' => {
                    tokens[count] = Token::Semicolon;
                    count += 1;
                    pos += 1;
                }
                _ if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-' => {
                    let start = pos;
                    while pos < bytes.len()
                        && (bytes[pos].is_ascii_alphanumeric()
                            || bytes[pos] == b'_'
                            || bytes[pos] == b'-')
                    {
                        pos += 1;
                    }
                    // SAFETY: We are slicing valid UTF-8 at ASCII character
                    // boundaries (alphanumeric, underscore, hyphen).
                    let word = &input[start..pos];
                    match word {
                        "allow" | "deny" | "type_transition" | "role" | "user" | "sensitivity"
                        | "category" => {
                            tokens[count] = Token::Keyword(word);
                        }
                        _ => {
                            tokens[count] = Token::Ident(word);
                        }
                    }
                    count += 1;
                }
                _ => {
                    // Skip unknown characters
                    pos += 1;
                }
            }
        }

        (tokens, count)
    }

    /// Parse a policy text and return parsed rules, transitions, roles,
    /// and user-role mappings.
    ///
    /// The input MUST be a `&'static str` (e.g., a const string literal or a
    /// string with static lifetime) so that the parsed `&'static str`
    /// references remain valid.
    pub(super) fn parse(input: &'static str) -> Result<ParsedPolicy, KernelError> {
        let (tokens, token_count) = Self::tokenize(input);
        let mut result = ParsedPolicy::new();

        let mut i = 0;
        while i < token_count {
            match tokens[i] {
                Token::Keyword(kw) => match kw {
                    "allow" | "deny" => {
                        let action = if kw == "allow" {
                            PolicyAction::Allow
                        } else {
                            PolicyAction::Deny
                        };
                        // allow source_type target_type { perm perm ... };
                        if i + 5 >= token_count {
                            return Err(KernelError::InvalidArgument {
                                name: "policy",
                                value: "incomplete allow/deny rule",
                            });
                        }
                        let source = Self::expect_ident(&tokens, token_count, i + 1)?;
                        let target = Self::expect_ident(&tokens, token_count, i + 2)?;
                        Self::expect_token_kind(&tokens, token_count, i + 3, Token::LBrace)?;

                        let mut perms = [Permission::Read; MAX_PERMISSIONS];
                        let mut perm_count: u8 = 0;
                        let mut j = i + 4;
                        while j < token_count {
                            match tokens[j] {
                                Token::RBrace => break,
                                Token::Ident(p) => {
                                    if (perm_count as usize) < MAX_PERMISSIONS {
                                        match p {
                                            "read" => {
                                                perms[perm_count as usize] = Permission::Read;
                                                perm_count += 1;
                                            }
                                            "write" => {
                                                perms[perm_count as usize] = Permission::Write;
                                                perm_count += 1;
                                            }
                                            "execute" => {
                                                perms[perm_count as usize] = Permission::Execute;
                                                perm_count += 1;
                                            }
                                            _ => {
                                                return Err(KernelError::InvalidArgument {
                                                    name: "permission",
                                                    value: "unknown permission",
                                                });
                                            }
                                        }
                                    }
                                    j += 1;
                                }
                                _ => {
                                    return Err(KernelError::InvalidArgument {
                                        name: "policy",
                                        value: "unexpected token in permission list",
                                    });
                                }
                            }
                        }
                        // j is now at RBrace
                        // expect semicolon after RBrace
                        if j + 1 < token_count && matches!(tokens[j + 1], Token::Semicolon) {
                            j += 1;
                        }
                        if result.rule_count < MAX_PARSED_RULES {
                            result.rules[result.rule_count] =
                                Some(PolicyRule::new(source, target, perms, perm_count, action));
                            result.rule_count += 1;
                        }
                        i = j + 1;
                    }
                    "type_transition" => {
                        // type_transition source target : class new_type ;
                        if i + 7 > token_count {
                            return Err(KernelError::InvalidArgument {
                                name: "policy",
                                value: "incomplete type_transition",
                            });
                        }
                        let source = Self::expect_ident(&tokens, token_count, i + 1)?;
                        let target = Self::expect_ident(&tokens, token_count, i + 2)?;
                        Self::expect_token_kind(&tokens, token_count, i + 3, Token::Colon)?;
                        let class = Self::expect_ident(&tokens, token_count, i + 4)?;
                        let new_type = Self::expect_ident(&tokens, token_count, i + 5)?;
                        // optional semicolon
                        let mut next = i + 6;
                        if next < token_count && matches!(tokens[next], Token::Semicolon) {
                            next += 1;
                        }
                        if result.transition_count < MAX_PARSED_TRANSITIONS {
                            result.transitions[result.transition_count] =
                                Some(DomainTransition::new(source, target, class, new_type));
                            result.transition_count += 1;
                        }
                        i = next;
                    }
                    "role" => {
                        // role name types { type1 type2 ... };
                        // simplified: role name { type1 type2 ... };
                        if i + 2 >= token_count {
                            return Err(KernelError::InvalidArgument {
                                name: "policy",
                                value: "incomplete role definition",
                            });
                        }
                        let name = Self::expect_ident(&tokens, token_count, i + 1)?;
                        // Skip optional "types" keyword
                        let mut j = i + 2;
                        if j < token_count {
                            if let Token::Ident(s) = tokens[j] {
                                if s == "types" {
                                    j += 1;
                                }
                            }
                        }
                        Self::expect_token_kind(&tokens, token_count, j, Token::LBrace)?;
                        j += 1;

                        let mut types = [""; MAX_ROLE_TYPES];
                        let mut type_count: usize = 0;
                        while j < token_count {
                            match tokens[j] {
                                Token::RBrace => break,
                                Token::Ident(t) => {
                                    if type_count < MAX_ROLE_TYPES {
                                        types[type_count] = t;
                                        type_count += 1;
                                    }
                                    j += 1;
                                }
                                _ => {
                                    return Err(KernelError::InvalidArgument {
                                        name: "policy",
                                        value: "unexpected token in role types",
                                    });
                                }
                            }
                        }
                        // j at RBrace
                        if j + 1 < token_count && matches!(tokens[j + 1], Token::Semicolon) {
                            j += 1;
                        }
                        if result.role_count < MAX_PARSED_ROLES {
                            result.roles[result.role_count] = Some(Role {
                                name,
                                allowed_types: types,
                                type_count,
                            });
                            result.role_count += 1;
                        }
                        i = j + 1;
                    }
                    "user" => {
                        // user username roles { role1 role2 ... };
                        if i + 2 >= token_count {
                            return Err(KernelError::InvalidArgument {
                                name: "policy",
                                value: "incomplete user definition",
                            });
                        }
                        let username = Self::expect_ident(&tokens, token_count, i + 1)?;
                        // Skip optional "roles" keyword
                        let mut j = i + 2;
                        if j < token_count {
                            if let Token::Ident(s) = tokens[j] {
                                if s == "roles" {
                                    j += 1;
                                }
                            }
                        }
                        Self::expect_token_kind(&tokens, token_count, j, Token::LBrace)?;
                        j += 1;

                        let mut assigned_roles = [""; MAX_USER_ASSIGNED_ROLES];
                        let mut role_count: usize = 0;
                        while j < token_count {
                            match tokens[j] {
                                Token::RBrace => break,
                                Token::Ident(r) => {
                                    if role_count < MAX_USER_ASSIGNED_ROLES {
                                        assigned_roles[role_count] = r;
                                        role_count += 1;
                                    }
                                    j += 1;
                                }
                                _ => {
                                    return Err(KernelError::InvalidArgument {
                                        name: "policy",
                                        value: "unexpected token in user roles",
                                    });
                                }
                            }
                        }
                        if j + 1 < token_count && matches!(tokens[j + 1], Token::Semicolon) {
                            j += 1;
                        }
                        if result.user_role_count < MAX_PARSED_USER_ROLES {
                            result.user_roles[result.user_role_count] = Some(UserRoleEntry {
                                username,
                                roles: assigned_roles,
                                role_count,
                            });
                            result.user_role_count += 1;
                        }
                        i = j + 1;
                    }
                    "sensitivity" | "category" => {
                        // These are declarative; skip to semicolon
                        i += 1;
                        while i < token_count && !matches!(tokens[i], Token::Semicolon) {
                            i += 1;
                        }
                        i += 1; // skip semicolon
                    }
                    _ => {
                        i += 1;
                    }
                },
                _ => {
                    i += 1;
                }
            }
        }

        Ok(result)
    }

    /// Extract an identifier at the given position.
    fn expect_ident<'a>(
        tokens: &[Token<'a>],
        token_count: usize,
        pos: usize,
    ) -> Result<&'a str, KernelError> {
        if pos >= token_count {
            return Err(KernelError::InvalidArgument {
                name: "policy",
                value: "unexpected end of policy",
            });
        }
        match tokens[pos] {
            Token::Ident(s) => Ok(s),
            _ => Err(KernelError::InvalidArgument {
                name: "policy",
                value: "expected identifier",
            }),
        }
    }

    /// Verify that the token at `pos` matches `expected` (structural tokens
    /// only).
    fn expect_token_kind(
        tokens: &[Token<'_>],
        token_count: usize,
        pos: usize,
        expected: Token<'_>,
    ) -> Result<(), KernelError> {
        if pos >= token_count {
            return Err(KernelError::InvalidArgument {
                name: "policy",
                value: "unexpected end of policy",
            });
        }
        if matches!(
            (&expected, &tokens[pos]),
            (Token::LBrace, Token::LBrace)
                | (Token::RBrace, Token::RBrace)
                | (Token::Colon, Token::Colon)
                | (Token::Semicolon, Token::Semicolon)
        ) {
            Ok(())
        } else {
            Err(KernelError::InvalidArgument {
                name: "policy",
                value: "unexpected token",
            })
        }
    }
}
