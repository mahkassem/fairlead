<?php

declare(strict_types=1);

namespace Fairlead\Composer;

/** A failure the launcher explains to the person running it, then exits 1. */
final class LauncherError extends \RuntimeException
{
}

/** The server answered 404: the release, or its build for this platform, doesn't exist. */
final class NotFound extends \RuntimeException
{
}
