; Runnable tasks in turbo.json - show "Run <task>" buttons
; Matches task definitions inside the "tasks" object
(
    (document
        (object
            (pair
                key: (string
                    (string_content) @_tasks_key
                    (#eq? @_tasks_key "tasks")
                )
                value: (object
                    (pair
                        key: (string (string_content) @run @task)
                    )
                )
            )
        )
    )
    (#set! tag turbo-task)
)
