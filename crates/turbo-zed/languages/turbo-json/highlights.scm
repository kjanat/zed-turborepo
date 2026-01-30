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

; Special turbo.json keys
(pair
  key: (string (string_content) @keyword)
  (#any-of? @keyword
    "tasks"
    "globalDependencies"
    "globalEnv"
    "globalPassThroughEnv"
    "extends"
    "experimentalUI"
    "daemon"
    "envMode"
    "cacheDir"
    "remoteCache"))

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
    "interactiveOutputLogs"
    "interactive"))

; Punctuation
"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
":" @punctuation.delimiter
"," @punctuation.delimiter
