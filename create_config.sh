#!/bin/bash
id=0
instances=$(kubectl get instances --no-headers -o custom-columns=":metadata.name")
mkdir configs || true
for instance in $instances; do
        echo "id: ${id}" > configs/${instance}.yaml
        echo "interfaces:" >> configs/${instance}.yaml
        interfaces=$(kubectl get interfaces -l virt.dev/instance=${instance} --no-headers -o custom-columns=":metadata.name")
        echo kubectl get interfaces -l virt.dev/instance=${instance} --no-headers -o custom-columns=":metadata.name"
        echo ${instance}
        for interface in $interfaces; do
                name=$(kubectl get interface ${interface} --no-headers -o custom-columns=":spec.name")
                echo "- ${name}" >> configs/${instance}.yaml
                ip=$(kubectl get interface ${interface} --no-headers -o custom-columns=":status.prefix")
                len=$(kubectl get interface ${interface} --no-headers -o custom-columns=":status.prefixLen")
                lxc exec ${instance} ip address add ${ip}/${len} dev ${name}
		lxc exec ${instance} ip link set dev ${name} up
        done
        lxc file push configs/${instance}.yaml ${instance}/
        ((id++))
done
networks=$(kubectl get networks --no-headers -o custom-columns=":metadata.name")
for network in $networks; do
        echo "id: ${id}" > configs/${network}.yaml
        echo "interfaces:" >> configs/${network}.yaml
        interfaces=$(kubectl get interfaces -l virt.dev/network=${network} --no-headers -o custom-columns=":metadata.name")
        for interface in $interfaces; do
                echo "- ${interface}" >> configs/${network}.yaml
        done
        ((id++))
done
