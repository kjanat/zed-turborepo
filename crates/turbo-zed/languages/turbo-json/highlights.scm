; Turbo JSON syntax highlighting
; Based on JSON grammar with turbo-specific enhancements

(comment) @comment

(string) @string

(number) @number

(true) @constant.builtin
(false) @constant.builtin
(null) @constant.builtin

; Object keys
(pair
  key: (string) @property)

; Special turbo.json root keys
(pair
  key: (string (string_content) @keyword)
  (#any-of? @keyword
    "$schema"
    "tasks"
    "globalDependencies"
    "globalEnv"
    "globalPassThroughEnv"
    "extends"
    "ui"
    "noUpdateNotifier"
    "concurrency"
    "dangerouslyDisablePackageManagerCheck"
    "daemon"
    "envMode"
    "cacheDir"
    "remoteCache"
    "futureFlags"
    "boundaries"
    "tags"
    "experimentalUI"
    "legacyExperiments"))

; Task configuration keys
(pair
  key: (string (string_content) @type)
  (#any-of? @type
    "dependsOn"
    "outputs"
    "inputs"
    "cache"
    "persistent"
    "env"
    "passThroughEnv"
    "outputLogs"
    "interactive"
    "interruptible"
    "with"
    "description"
    "allowAllOutputLogsOnSuccess"))

; Remote cache configuration keys
(pair
  key: (string (string_content) @type)
  (#any-of? @type
    "enabled"
    "signature"
    "preflight"
    "timeout"
    "uploadTimeout"
    "apiUrl"
    "loginUrl"
    "teamId"
    "teamSlug"))

; Boundaries configuration keys
(pair
  key: (string (string_content) @type)
  (#any-of? @type
    "dependencies"
    "dependents"
    "allow"
    "deny"
    "taskOverrides"))

; Future flags keys
(pair
  key: (string (string_content) @type)
  (#any-of? @type
    "errorsOnlyShowHash"))

; Special turbo microsyntax values (in strings)
(string
  (string_content) @constant.builtin
  (#any-of? @constant.builtin
    "$TURBO_EXTENDS$"
    "$TURBO_DEFAULT$"
    "//"))

; Root-relative path prefix
(string
  (string_content) @constant.builtin
  (#match? @constant.builtin "^\\$TURBO_ROOT\\$"))

; Dependency relationship prefix (^task)
(string
  (string_content) @operator
  (#match? @operator "^\\^"))

; Negation prefix (! for globs and env)
(string
  (string_content) @operator
  (#match? @operator "^!"))

; Package#task syntax
(string
  (string_content) @function
  (#match? @function "^[a-zA-Z0-9@/_-]+#[a-zA-Z0-9_-]+$"))

; Punctuation
"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
":" @punctuation.delimiter
"," @punctuation.delimiter
