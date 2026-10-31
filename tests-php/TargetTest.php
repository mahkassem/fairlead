<?php

declare(strict_types=1);

use Fairlead\Composer\Target;

test('each platform maps to the release build that runs on it', function (): void {
    $glibc = ['family' => 'glibc', 'version' => '2.39'];
    $musl = ['family' => 'musl'];
    $table = [
        ['Linux', 'x86_64', $glibc, 'x86_64-unknown-linux-gnu'],
        ['Linux', 'aarch64', $glibc, 'aarch64-unknown-linux-gnu'],
        ['Linux', 'arm64', $glibc, 'aarch64-unknown-linux-gnu'],
        ['Linux', 'x86_64', $musl, 'x86_64-unknown-linux-musl'],
        ['Linux', 'aarch64', $musl, 'aarch64-unknown-linux-musl'],
        // glibc older than the glibc build needs, an unreadable version, or no answer at all
        ['Linux', 'x86_64', ['family' => 'glibc', 'version' => '2.31'], 'x86_64-unknown-linux-musl'],
        ['Linux', 'x86_64', ['family' => 'glibc', 'version' => '2.35'], 'x86_64-unknown-linux-gnu'],
        ['Linux', 'x86_64', ['family' => 'glibc', 'version' => null], 'x86_64-unknown-linux-gnu'],
        ['Linux', 'x86_64', null, 'x86_64-unknown-linux-musl'],
        ['Darwin', 'x86_64', null, 'x86_64-apple-darwin'],
        ['Darwin', 'arm64', null, 'aarch64-apple-darwin'],
        ['Windows', 'AMD64', null, 'x86_64-pc-windows-msvc'],
        ['Windows', 'ARM64', null, 'x86_64-pc-windows-msvc'],
        ['Linux', 'armv7l', $glibc, null],
        ['Linux', 'i686', $glibc, null],
        ['Linux', 'riscv64', $glibc, null],
        ['BSD', 'amd64', null, null],
        ['Solaris', 'x86_64', null, null],
        ['Windows', 'x86', null, null],
    ];
    foreach ($table as [$family, $machine, $libc, $expected]) {
        eq($expected, Target::triple($family, $machine, $libc), "{$family} {$machine} " . json_encode($libc));
    }
});

test('the archive and binary names follow the release assets', function (): void {
    eq('fairlead-x86_64-unknown-linux-gnu.tar.xz', Target::archive('x86_64-unknown-linux-gnu'));
    eq('fairlead-aarch64-apple-darwin.tar.xz', Target::archive('aarch64-apple-darwin'));
    eq('fairlead-x86_64-pc-windows-msvc.zip', Target::archive('x86_64-pc-windows-msvc'));
    eq('fairlead.exe', Target::binary('x86_64-pc-windows-msvc'));
    eq('fairlead', Target::binary('x86_64-unknown-linux-musl'));
});

test('the C library is read from ldd first, then the musl loader', function (): void {
    $none = static fn (): array => [];
    $loader = static fn (): array => ['/lib/ld-musl-x86_64.so.1'];
    $ldd = static fn (?string $out): Closure => static fn (): ?string => $out;
    $ubuntu = "ldd (Ubuntu GLIBC 2.39-0ubuntu8.9) 2.39\nCopyright (C) 2024 Free Software Foundation, Inc.\n";
    $alpine = "musl libc (x86_64)\nVersion 1.2.4\nDynamic Program Loader\n";
    eq(['family' => 'glibc', 'version' => '2.39'], Target::libc($none, $ldd($ubuntu)));
    eq(['family' => 'glibc', 'version' => '2.36'], Target::libc($none, $ldd("ldd (Debian GLIBC 2.36-9+deb12u4) 2.36\n")));
    eq(['family' => 'glibc', 'version' => '2.38'], Target::libc($none, $ldd("ldd (GNU libc) 2.38\n")));
    eq(['family' => 'musl'], Target::libc($none, $ldd($alpine)));
    // musl installed beside glibc puts its loader in /lib; ldd still says glibc.
    eq(['family' => 'glibc', 'version' => '2.39'], Target::libc($loader, $ldd($ubuntu)));
    eq(['family' => 'musl'], Target::libc($loader, $ldd(null)));
    eq(null, Target::libc($none, $ldd(null)));
});
