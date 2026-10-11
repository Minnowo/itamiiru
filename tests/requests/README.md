Sample picker requests (protocol v1, see PLAN.md section 5).

    cargo run -p itamiiru -- --pick < tests/requests/open-images.req; echo "exit: $?"

Expected stdout on success, e.g.:

    itamiiru-protocol 1
    uri file:///home/me/Pictures/cat.png
    filter 0

Exit code 0 = OK, 1 = cancelled, other = error.
