KUBE_CONTEXT ?= docker-desktop
METRICS_NAMESPACE ?= kube-system
METRICS_RELEASE ?= metrics-server
METRICS_CHART ?= metrics-server/metrics-server
METRICS_REPO_NAME ?= metrics-server
METRICS_REPO_URL ?= https://kubernetes-sigs.github.io/metrics-server/

.PHONY: metrics-api-install metrics-api-status metrics-api-top

metrics-api-install:
	rtk helm repo add $(METRICS_REPO_NAME) $(METRICS_REPO_URL)
	rtk helm repo update $(METRICS_REPO_NAME)
	rtk helm upgrade --install $(METRICS_RELEASE) $(METRICS_CHART) \
		--kube-context $(KUBE_CONTEXT) \
		--namespace $(METRICS_NAMESPACE) \
		--set 'args[0]=--kubelet-insecure-tls' \
		--set 'args[1]=--kubelet-preferred-address-types=InternalIP\,Hostname\,ExternalIP'
	rtk kubectl --context $(KUBE_CONTEXT) -n $(METRICS_NAMESPACE) rollout status deployment/$(METRICS_RELEASE) --timeout=120s

metrics-api-status:
	rtk kubectl --context $(KUBE_CONTEXT) -n $(METRICS_NAMESPACE) get deployment $(METRICS_RELEASE)
	rtk kubectl --context $(KUBE_CONTEXT) get apiservice v1beta1.metrics.k8s.io

metrics-api-top:
	rtk kubectl --context $(KUBE_CONTEXT) -n danske-spil top pods
