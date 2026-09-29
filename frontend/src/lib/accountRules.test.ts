import { describe, expect, it } from 'vitest';
import {
  emailProblem,
  isBareAddress,
  problems,
  requiredProblem,
  serverTrim,
  usernameProblem
} from './accountRules';

describe('F-ACCOUNT-1 the email rule is the server’s is_bare_address', () => {
  // The cases of `routes::auth::tests::only_a_bare_address_is_an_email`.
  it('takes what the server takes', () => {
    for (const good of [
      'a@example.com',
      'first.last+tag@mail.example.co.uk',
      'x_y-z@b.io',
      "o'neil@x-y.example",
      '1@123.example'
    ]) {
      expect(isBareAddress(good), good).toBe(true);
    }
  });

  it('refuses what the server refuses, a@b included', () => {
    for (const bad of [
      'x <victim@example.com>', 'victim@example.com,other@x.com', 'a@b', '@example.com',
      'a@@example.com', 'a@example..com', 'a b@example.com', 'a@example.com.', '"a"@example.com',
      'a@.example.com', 'a;b@example.com', 'é@example.com',
      '.a@example.com', 'a.@example.com', 'a..b@example.com', 'a@exa_mple.com', 'a@ex!ample.com',
      'a@-example.com', 'a@example-.com', 'a@1.2.3.4', 'a@example.com\u0000',
      // No trailing-newline leniency, as a `$` without the `m` flag might suggest.
      'a@example.com\n'
    ]) {
      expect(isBareAddress(bad), bad).toBe(false);
    }
  });

  it('holds the lengths: 64 before the @, 63 a label, 254 in all', () => {
    expect(isBareAddress(`${'a'.repeat(64)}@example.com`)).toBe(true);
    expect(isBareAddress(`${'a'.repeat(65)}@example.com`)).toBe(false);
    expect(isBareAddress(`a@${'b'.repeat(63)}.com`)).toBe(true);
    expect(isBareAddress(`a@${'b'.repeat(64)}.com`)).toBe(false);
    const long = `ab@${Array(4).fill('b'.repeat(61)).join('.')}.com`; // 254 characters
    expect(long.length).toBe(254);
    expect(isBareAddress(long)).toBe(true);
    expect(isBareAddress(`a${long}`)).toBe(false);
  });

  it('checks the address trimmed and lower-cased, as the server stores it', () => {
    expect(emailProblem('  Alice@Example.ORG \n')).toBeNull();
    expect(emailProblem('alice@')).toBe('must be a valid email address');
    expect(emailProblem('')).toBe('must be a valid email address');
  });
});

describe('F-ACCOUNT-2 the username length rule counts as the server does', () => {
  it('takes 3 to 32 characters after trimming', () => {
    expect(usernameProblem('abc')).toBeNull();
    expect(usernameProblem('a'.repeat(32))).toBeNull();
    expect(usernameProblem('ab')).toBe('must be between 3 and 32 characters');
    expect(usernameProblem('a'.repeat(33))).toBe('must be between 3 and 32 characters');
    expect(usernameProblem('  ab  ')).toBe('must be between 3 and 32 characters');
    expect(usernameProblem('')).toBe('must be between 3 and 32 characters');
  });

  it('counts code points, not UTF-16 units', () => {
    // Two astral characters are four units: too short all the same.
    expect(usernameProblem('😀😀')).toBe('must be between 3 and 32 characters');
    expect(usernameProblem('😀'.repeat(32))).toBeNull();
  });

  it('trims Rust’s white space, not JavaScript’s', () => {
    // U+0085 is white space to Rust and not to `String.prototype.trim`;
    // U+FEFF the other way round.
    expect(serverTrim('\u0085ab\u0085')).toBe('ab');
    expect(serverTrim('\u3000ab\u2029')).toBe('ab');
    expect(serverTrim('\uFEFFab')).toBe('\uFEFFab');
    expect(usernameProblem('\uFEFFab')).toBeNull();
  });
});

describe('F-ACCOUNT-3 required fields and the problems map', () => {
  it('finds an empty field, trimming all but a password', () => {
    expect(requiredProblem('')).toBe('must not be empty');
    expect(requiredProblem('  ')).toBe('must not be empty');
    expect(requiredProblem('  ', false)).toBeNull();
    expect(requiredProblem('x')).toBeNull();
  });

  it('keeps only the fields with a problem', () => {
    expect(problems({ username: null, email: 'must be a valid email address' })).toEqual({
      email: 'must be a valid email address'
    });
    expect(problems({ username: null })).toEqual({});
  });
});
