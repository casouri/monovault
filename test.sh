#!/bin/bash

max_client=2
configs=("config.json" "config1.json" "config2.json")
mount_dirs=("mount" "mount2" "mount3")
work_dirs=("pandora" "moon" "laogong")
client_logs=("0.log" "1.log" "2.log")
pids=("-1" "-1" "-1")

test_str1="helloworrdddd..."
test_str2="qwertyuiop...."

function err() {
    echo "> $@" >&2
}

function msg {
    echo "> $@"
}

function invalid_client_id {
    local re='^[0-9]+$'
    if ! [[ $1 =~ $re ]] ; then
        return 0
    fi
    if [[ "$1" -lt 0 || "$1" -gt "$max_client" ]]; then
        return 0
    fi
    return 1
}

function start_client {
    if invalid_client_id $1; then
        err "Invalid client ID $1, abort..."
        exit -1
    fi
    cargo run -- -c "${configs[$1]}" > "${client_logs[$1]}" 2>&1 &
    pids[$1]=$!
    msg "start client $1 pid ${pids[$1]} ..."
}

function stop_client {
    if invalid_client_id $1; then
        err "Invalid client ID $1, abort..."
        exit -1
    fi
    msg "kill client '${pids[$1]}' ..."
    kill ${pids[$1]} &>/dev/null
    umount ${mount_dirs[$1]} &>/dev/null
}

# para1: caller ID, para2: target ID
function get_dir {
    if invalid_client_id $1 || invalid_client_id $2; then
        err "Invalid client ID $1 or $2, abort.."
        exit -1
    fi
    echo "${mount_dirs[$1]}/${work_dirs[$2]}"
}

# para1: mount ID, para2: client directory, para3: write string
function write_file {
    local dir=$(get_dir $1 $2)
    cd $dir
    msg "client $1 write str [$3] in $dir"
    echo $3 > tsfile
    cd ../..
    msg "wait for sync..."
    sleep 5
}

function read_file {
    local dir=$(get_dir $1 $2)
    cd $dir
    cat tsfile
    cd ../..
}

#A B两个host，A读B的文件，B断线，A依然可以读写，B上线以后A的修改会同步到B
function test1 {
    start_client 0
    start_client 1
    sleep 10 # wait for client start up

    write_file 1 1 $test_str1
    local result=$(read_file 0 1)
    msg "client 0 read str [$result]"
    if [[ "$result" != "$test_str1" ]]; then
        err "***** test1 failed!!!!"
        return
    fi

    stop_client 1
    sleep 5
    result=$(read_file 0 1)
    msg "client 0 read str [$result]"
    if [[ "$result" != "$test_str1" ]]; then
        err "***** test1 failed!!!!"
        return
    fi

    write_file 0 1 $test_str2

    start_client 1
    sleep 10
    result=$(read_file 1 1)
    msg "client 1 read str [$result]"
    if [[ "$result" != "$test_str2" ]]; then
        err "***** test1 failed!!!!"
        return
    fi

    msg "***** test1 pass!"
}

function main {
    test1

    for i in $(seq 0 $max_client); do
        stop_client $i
    done
}

main $@
