use std::net::{TcpStream, TcpListener};
use std::{env, thread, time, io, fs};
use std::io::{BufRead, Write, BufReader, Error, ErrorKind, Read};
use std::process::Command;
use fork::{Fork, daemon};
use std::fs::File;
use std::cmp::min;
use std::sync::atomic::{AtomicUsize, Ordering};


fn download(file: &String, path: &String){
    //aria2c -c $downloadList --connect-timeout=1 -s $maxMirrorForDownload -t 1 -d "$pacmanCahceDir" &> /dev/null
    let slice = &path[1..];
    let output = Command::new("aria2c").args(&["-c", file, "--connect-timeout=1", "-s1", "-t1", "-d/", "-o", slice]).output().expect("failed to execute process");;
    if !output.status.success() {
        println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        Command::new("notify-send").arg("'downloading failed with ".to_owned() + &String::from_utf8_lossy(&output.stdout)+"'").output();
    }
}

fn read_incoming_packet(listener: &TcpListener, packets: &mut Vec<(String, String)>, clients: &mut Vec<BufReader<TcpStream>>, expected: &mut usize){
    // accept connections
    //let asd = listener.incoming().iter();
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                println!("New connection: {}", stream.peer_addr().unwrap());
                match stream.set_nonblocking(true){
                    Ok(_) => {
                        clients.push(BufReader::new(stream));
                    },
                    Err(e) => {
                        println!("Cannot set non-blocking client {}", e);
                    }
                }


            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                break;
            }
            Err(e) => {
                println!("Error: {:?}", e);
                break;
            }
        }
    }

    // read lines
    let mut i = 0;
    while i != clients.len() {

        let mut line = String::new();
        let res = (&mut clients[i]).read_line(&mut line);

        match res {
            Ok(len) => {
                if len == 0{
                    println!("client disconnected {}", clients[i].get_ref().peer_addr().unwrap());
                    clients.remove(i);
                }else {
                    let split: Vec<&str> = line.split(",").collect();
                    if split.len() == 2 {
                        println!("got {} {}", split[0], split[1]);
                        packets.push((split[0].to_owned(), split[1].to_owned()));
                    } else {
                        println!("error splitting received line: {}", line);
                    }
                    if *expected > 1 {
                        *expected -= 1;
                        println!("disconnecting client {}", clients[i].get_ref().peer_addr().unwrap());
                        clients.remove(i);
                    }else if *expected == 1 {
                        *expected -= 1;
                        println!("keeping alive last client!!!");
                    }else{
                        println!("More download than expected requested");
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                println!("wouldblock socket: {}", e);
                i += 1;
            }
            Err(e) => {
                println!("error reading socket: {}", e);
                clients.remove(i);
            }
        }
    }
}

static GLOBAL_THREAD_COUNT: AtomicUsize = AtomicUsize::new(0);

fn download2(packet: (String, String)) -> Result<(), Error> {
    download(&packet.0, &packet.1);
    match packet.1.rfind('.'){
        None => {return Err(Error::new(ErrorKind::Other,"no point found"));},
        Some(index) => {
            println!("renaming {} to {}", packet.1, &packet.1[..index]);
            fs::rename(&packet.1, &packet.1[..index])?;
        },
    }
    Ok(())
}

fn download_packet(mut expected: usize) {
    let mut packets = Vec::new();
    let listener = TcpListener::bind("0.0.0.0:6663").unwrap();
    let mut clients = Vec::new();
    listener.set_nonblocking(true).expect("Cannot set non-blocking");
    println!("Server listening on port 6663");

    let max_count: usize = 10;
    while packets.len() > 0 || expected > 0 {
        read_incoming_packet(&listener, &mut packets, &mut clients, &mut expected);

        for _x in 0..min(packets.len(), max_count - GLOBAL_THREAD_COUNT.load(Ordering::Relaxed)) {
            let packet = packets.remove(0);
            let current_thread = GLOBAL_THREAD_COUNT.fetch_add(1, Ordering::SeqCst);
            println!("starting download of {} thread alive before creation {}", packet.1, current_thread);

            thread::spawn(move || {
                download2(packet);
                GLOBAL_THREAD_COUNT.fetch_sub(1, Ordering::SeqCst);
            });
        }
        println!("still expecting {} packet request, waiting {} thread and {} packet", expected, GLOBAL_THREAD_COUNT.load(Ordering::Relaxed), packets.len());
        thread::sleep(time::Duration::from_secs(1));
    }
}

fn server() -> Result<(), Error> {
    println!("server!");

    let mut expected = 0;
    let output = Command::new("checkupdates").output()?;


    if !output.status.success() {
        println!("status: {}", output.status);
        println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err(Error::new(ErrorKind::Other, "Command executed with failing error code"));
    }else{
        match String::from_utf8(output.stdout) {
            Ok(s) => {
                for _l in s.lines() {
                    expected += 1;
                }
            },
            Err(_) => {
                return Err(Error::new(ErrorKind::Other, "Command executed with failing error code"));
            },
        }
    }

    if expected == 0{
        if let Ok(Fork::Child) = daemon(false, false) {
            download_packet(expected);
        }
    }else{
        let args: Vec<_> = env::args().collect();
        if args.len() == 2 {
            println!("direct download {} {}", &args[1], &args[2]);
            download(&args[1], &args[2]);
        }
    }


    return Ok(());
}

fn main() -> Result<(), Error>{

    let args: Vec<_> = env::args().collect();
    match TcpStream::connect("localhost:6664") {
        Ok(mut stream) => {
            stream.write( format!("'download request {:?}'\n", args).as_ref() )?;
        },
        Err(_e) => {

        }
    }
    if args.len() != 3 {
        println!("param: {:?}", args);
        return Err(Error::new(ErrorKind::Other, "Parameter number incorrect"));
    }
    println!("got {} {}", &args[1], &args[2]);

    if args[2].ends_with(".db.sig.part"){
        return Ok(());
    }

    if args[2].ends_with(".db.part"){
        //download immediately
        println!("direct download {} {}", &args[1], &args[2]);

        Command::new("notify-send").arg("'downloading ".to_owned() + &args[1] + "'").output()?;
        download(&args[1], &args[2]);
    }else {

        //create fake file
        match File::create(&args[2]) {
            Ok(_) => { println!("created file {}", &args[2]); },
            Err(e) => { println!("error creating temp file: {}", e); },
        }
        //try to send the file to the server, or become the server
        match TcpStream::connect("localhost:6663") {
            Ok(mut stream) => {
                let message: String = args[1].to_owned() + "," + &args[2] + "\n";

                stream.write(message.as_ref())?;
                println!("sent file for download");
                thread::sleep(time::Duration::from_secs(1));

                let mut buffer:[u8;1] = [0];
                println!("socket reading");
                stream.read(&mut buffer);
                println!("socket has been closed");
            },
            Err(_e) => {
                server()?;
            }
        }
    }
    println!("Terminated.");
    Ok(())
}